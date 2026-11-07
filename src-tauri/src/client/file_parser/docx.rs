use super::FileParser;
use crate::error::Result;
use async_trait::async_trait;
use bytes::Bytes;
use docx_rs::read_docx;
use serde_json::Value;

struct DocxParser {}

fn read_children(node: &Value, text: &mut String) {
    // docx-rs serializes tables and rows under different child collection
    // names. Only visit content collections, not properties or relationships.
    let child_key = match node["type"].as_str() {
        Some("table") => "rows",
        Some("tableRow") => "cells",
        _ => "children",
    };
    if let Some(children) = node["data"][child_key].as_array() {
        children.iter().for_each(|child| {
            if child["type"] != "text" {
                read_children(child, text);
            } else {
                let mut paragraph = child["data"]["text"]
                    .as_str()
                    .unwrap_or_default()
                    .to_owned();
                paragraph.push('\n');
                text.push_str(&paragraph);
            }
        });
    }
}

#[async_trait]
impl FileParser for DocxParser {
    async fn parse(&self, data: Bytes) -> Result<String> {
        let docx = read_docx(&data)?;
        let data: Value = serde_json::from_str(&docx.json())?;
        let mut text = String::new();
        if let Some(children) = data["document"]["children"].as_array() {
            children
                .iter()
                .for_each(|node| read_children(node, &mut text));
        }
        Ok(text)
    }
}

pub fn new() -> Box<dyn FileParser> {
    Box::new(DocxParser {})
}

#[cfg(test)]
mod tests {
    use super::read_children;
    use serde_json::{json, Value};

    fn text(value: &str) -> Value {
        json!({"type": "text", "data": {"text": value}})
    }

    fn paragraph(value: &str) -> Value {
        json!({"type": "paragraph", "data": {"children": [
            {"type": "run", "data": {"children": [text(value)]}}
        ]}})
    }

    fn cell(children: Vec<Value>) -> Value {
        json!({"type": "tableCell", "data": {"children": children}})
    }

    fn table(cells: Vec<Value>) -> Value {
        json!({"type": "table", "data": {"rows": [
            {"type": "tableRow", "data": {"cells": cells}}
        ]}})
    }

    #[test]
    fn table_cells_and_nested_tables_preserve_reading_order() {
        let node = table(vec![
            cell(vec![paragraph("first"), table(vec![cell(vec![paragraph("nested")])])]),
            cell(vec![paragraph("last")]),
        ]);
        let mut result = String::new();
        read_children(&node, &mut result);
        assert_eq!(result, "first\nnested\nlast\n");
    }

    #[test]
    fn paragraph_run_text_stays_unchanged() {
        let mut result = String::new();
        read_children(&paragraph("课程 & <safe>"), &mut result);
        assert_eq!(result, "课程 & <safe>\n");
    }

    #[test]
    fn metadata_is_not_treated_as_document_text() {
        let mut node = table(vec![cell(vec![paragraph("content")])]);
        node["data"]["property"] = json!({"children": [text("not document text")]});
        let mut result = String::new();
        read_children(&node, &mut result);
        assert_eq!(result, "content\n");
    }

    #[test]
    fn missing_or_wrong_shape_collections_are_empty() {
        for node in [json!(null), json!({"type": "table"}),
            json!({"type": "tableRow", "data": {"cells": "not an array"}})] {
            let mut result = String::new();
            read_children(&node, &mut result);
            assert!(result.is_empty());
        }
    }
}
