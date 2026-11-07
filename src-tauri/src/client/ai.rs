use super::Client;
use crate::{
    error::Result,
    model::{CanvasVideoSubTitle, File, LLMChatMessage},
    utils::time::format_time,
};
use std::path::Path;

/// Non-queuing MCP file-work admission. Keep ownership through parsing/model use;
/// an abandoned blocking parser must continue to hold its slot until it exits.
/// This bounds admitted requests, not allocations inside metadata or parsers.
pub(crate) struct McpFileAdmission {
    _permit: tokio::sync::SemaphorePermit<'static>,
}

impl McpFileAdmission {
    pub(crate) fn try_acquire() -> Result<Self> {
        static FILE_SLOT: tokio::sync::Semaphore = tokio::sync::Semaphore::const_new(1);
        FILE_SLOT
            .try_acquire()
            .map(|permit| Self { _permit: permit })
            .map_err(|_| crate::error::AppError::LLMError(
                "MCP file explanation is busy; try again later".to_owned(),
            ))
    }
}

impl Client {
    pub async fn chat<S: Into<String>>(&self, prompt: S) -> Result<String> {
        self.llm_cli.chat(prompt.into()).await
    }

    async fn read_file_content(&self, file: &File) -> Result<String> {
        let path = Path::new(&file.display_name);
        let ext = path
            .extension()
            .and_then(|os_str| os_str.to_str())
            .unwrap_or("");
        let resp = self.get_request(&file.url, None::<&str>).await?;
        let data = resp.bytes().await?;
        let text = self.file_parser.parse(data, ext).await?;
        Ok(text)
    }

    fn default_file_summary_request(file_name: &str) -> String {
        format!(
            "请先总结这份文件。若它与作业相关，请额外列出得分点、提交要求、截止时间、文件格式限制和任何容易遗漏的注意事项。请使用 Markdown 输出，不要把整段内容包在代码块里。文件名：{file_name}"
        )
    }

    fn build_file_chat_prompt(&self, file: &File, text: &str, messages: &[LLMChatMessage]) -> String {
        let history = messages
            .iter()
            .map(|message| {
                let speaker = if message.role.eq_ignore_ascii_case("assistant") {
                    "AI"
                } else {
                    "User"
                };
                format!("{speaker}: {}", message.content.trim())
            })
            .collect::<Vec<_>>()
            .join("\n\n");

        format!(
            "你是一个大学课程助教，正在围绕同一份课程文件与学生持续对话。请优先依据文件内容回答；如果问题超出文件内容，请明确说明这一点，再给出最接近的帮助。回答请使用 Markdown，但不要把整段回答包在代码块中。\n\n文件名：{}\n\n文件内容：\n{}\n\n对话历史：\n{}",
            file.display_name, text, history
        )
    }

    pub async fn chat_with_file(&self, file: &File, messages: &[LLMChatMessage]) -> Result<String> {
        let text = self.read_file_content(file).await?;
        let prompt = self.build_file_chat_prompt(file, &text, messages);
        tracing::info!("File chat prompt prepared");
        self.llm_cli.chat(prompt).await
    }

    pub async fn chat_with_file_stream(
        &self,
        file: &File,
        messages: &[LLMChatMessage],
        on_chunk: &mut (dyn FnMut(String) + Send),
    ) -> Result<String> {
        let text = self.read_file_content(file).await?;
        let prompt = self.build_file_chat_prompt(file, &text, messages);
        tracing::info!("File streaming chat prompt prepared");
        self.llm_cli.chat_stream(prompt, on_chunk).await
    }

    pub async fn explain_file(&self, file: &File) -> Result<String> {
        let messages = vec![LLMChatMessage {
            role: "user".to_string(),
            content: Self::default_file_summary_request(&file.display_name),
        }];
        self.chat_with_file(file, &messages).await
    }

    /// MCP-only entry point. `file` must be freshly read from Canvas by App.
    /// The caller supplies an owner check that is polled after parsing and before
    /// model use. It is not an atomic revocation of an already issued request.
    pub(crate) async fn explain_file_for_mcp(
        &self,
        file: &File,
        admission: McpFileAdmission,
        before_model: impl std::future::Future<Output = Result<()>>,
    ) -> Result<String> {
        use super::mcp_download;
        use crate::error::AppError;
        let failure = |message: &str| AppError::LLMError(message.to_owned());
        let extension = mcp_download::validate_metadata(
            file.locked,
            file.locked_for_user,
            file.size,
            file.external_type.is_some(),
            &file.display_name,
        )
        .map_err(|error| failure(&error.to_string()))?;
        let data = mcp_download::download(&file.url, file.size)
            .await
            .map_err(|error| failure(&error.to_string()))?;

        // Existing PDF/DOCX parsers do synchronous CPU work. Keep it off Tokio
        // workers and bound waiting, with one parser slot retained by the worker
        // even if this request is cancelled or times out. This is not a sandbox:
        // a running spawn_blocking parser cannot be killed by dropping its handle.
        static PARSER_SLOT: tokio::sync::Semaphore = tokio::sync::Semaphore::const_new(1);
        let (text, _admission) = tokio::time::timeout(std::time::Duration::from_secs(15), async move {
            let permit = PARSER_SLOT
                .acquire()
                .await
                .map_err(|_| failure("MCP file parser is unavailable"))?;
            let runtime = tokio::runtime::Handle::current();
            tokio::task::spawn_blocking(move || {
                let _permit = permit;
                let text = runtime.block_on(
                    super::file_parser::new_generic_file_reader().parse(data.into(), &extension),
                ).map_err(|_| ())?;
                // Return admission with the text so model work stays bounded.
                // If the caller left, the worker/output retains it until done.
                Ok::<_, ()>((text, admission))
            })
            .await
            .map_err(|_| failure("MCP file parsing failed"))?
            .map_err(|_| failure("MCP file parsing failed"))
        })
        .await
        .map_err(|_| failure("MCP file parsing timed out"))??;
        mcp_download::validate_text(&text).map_err(|error| failure(&error.to_string()))?;
        before_model.await?;
        let messages = [LLMChatMessage {
            role: "user".to_owned(),
            content: Self::default_file_summary_request(&file.display_name),
        }];
        // App uses an isolated client with a frozen copy of the original LLM
        // state, never a subsequently configured account/provider.
        self.llm_cli
            .chat(self.build_file_chat_prompt(file, &text, &messages))
            .await
            .map_err(|_| failure("MCP file explanation failed"))
    }

    pub fn compress_subtitle(&self, subtitle: &[CanvasVideoSubTitle]) -> Result<String> {
        let mut result = String::new();

        let mut current_sentence = String::new();
        let mut sentence_start_time: Option<u64> = None;

        for item in subtitle {
            if sentence_start_time.is_none() {
                sentence_start_time = Some(item.bg);
            }

            current_sentence.push_str(item.res.trim());

            if current_sentence.ends_with('.') || current_sentence.ends_with('。') {
                let start_time = format_time(sentence_start_time.unwrap());

                result.push_str(&format!(
                    "[{}] {} ",
                    start_time,
                    current_sentence.trim()
                ));

                current_sentence.clear();
                sentence_start_time = None;
            }
        }

        if !current_sentence.is_empty() {
            let start_time = format_time(sentence_start_time.unwrap());
            result.push_str(&format!(
                "[{}] {}",
                start_time,
                current_sentence.trim()
            ));
        }

        Ok(result.trim().to_string())
    }

    fn default_subtitle_summary_request() -> String {
        "请先总结这节课的核心内容。重点关注课程活动与通知、作业/小测/考试/签到提醒，以及主要知识点与框架；如果合适，请引用对应的字幕时间点。请使用 Markdown 输出。".to_string()
    }

    fn build_subtitle_chat_prompt(
        &self,
        compressed_subtitle: &str,
        messages: &[LLMChatMessage],
    ) -> String {
        let history = messages
            .iter()
            .map(|message| {
                let speaker = if message.role.eq_ignore_ascii_case("assistant") {
                    "AI"
                } else {
                    "User"
                };
                format!("{speaker}: {}", message.content.trim())
            })
            .collect::<Vec<_>>()
            .join("\n\n");

        format!(
            "你是一个大学课程助教，正在围绕一段课程视频字幕与学生持续对话。请优先依据字幕回答；如果问题超出字幕，请明确说明。回答请使用 Markdown。字幕中包含一些时间点，格式为 `[HH:MM:SS,FFF]`；当你引用时间点时，请保留这个格式，方便前端跳转。\n\n视频字幕：\n{}\n\n对话历史：\n{}",
            compressed_subtitle, history
        )
    }

    pub async fn chat_with_subtitle(
        &self,
        canvas_course_id: i64,
        messages: &[LLMChatMessage],
    ) -> Result<String> {
        let subtitle = &self.get_subtitle(canvas_course_id).await?.before_assembly_list;
        let compressed_subtitle = self.compress_subtitle(subtitle)?;
        let prompt = self.build_subtitle_chat_prompt(&compressed_subtitle, messages);
        tracing::info!("Subtitle chat prompt prepared");
        self.llm_cli.chat(prompt).await
    }

    pub async fn chat_with_subtitle_stream(
        &self,
        canvas_course_id: i64,
        messages: &[LLMChatMessage],
        on_chunk: &mut (dyn FnMut(String) + Send),
    ) -> Result<String> {
        let subtitle = &self.get_subtitle(canvas_course_id).await?.before_assembly_list;
        let compressed_subtitle = self.compress_subtitle(subtitle)?;
        let prompt = self.build_subtitle_chat_prompt(&compressed_subtitle, messages);
        tracing::info!("Subtitle streaming chat prompt prepared");
        self.llm_cli.chat_stream(prompt, on_chunk).await
    }

    pub async fn summarize_subtitle(&self, canvas_course_id: i64) -> Result<String> {
        let messages = vec![LLMChatMessage {
            role: "user".to_string(),
            content: Self::default_subtitle_summary_request(),
        }];
        self.chat_with_subtitle(canvas_course_id, &messages).await
    }
}

#[cfg(test)]
mod mcp_file_tests {
    use super::*;

    // Both tests touch the process-wide admission slot.
    static TEST_LOCK: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());

    #[tokio::test]
    async fn mcp_file_admission_is_non_queuing_and_reusable() {
        let _test_guard = TEST_LOCK.lock().await;
        let admission = McpFileAdmission::try_acquire().unwrap();
        let error = McpFileAdmission::try_acquire().err().expect("second request must be busy");
        assert!(error.to_string().contains("busy"));
        drop(admission);
        assert!(McpFileAdmission::try_acquire().is_ok());
    }


    #[tokio::test]
    async fn untrusted_files_fail_before_parser_owner_guard_or_model() {
        let _test_guard = TEST_LOCK.lock().await;
        let client = Client::new_without_proxy("https://oc.sjtu.edu.cn", "", "", "", None);
        let metadata = File {
            display_name: "file.pdf".to_owned(),
            url: "https://oc.sjtu.edu.cn/files/1/download".to_owned(),
            locked_for_user: Some(false),
            size: 4,
            ..Default::default()
        };
        for file in [
            File { locked: true, ..metadata.clone() },
            File { locked_for_user: None, ..metadata.clone() },
            File { locked_for_user: Some(true), ..metadata.clone() },
            File { size: super::super::mcp_download::MAX_BYTES as u64 + 1, ..metadata.clone() },
            File { url: "file:///tmp/private.pdf".to_owned(), ..metadata.clone() },
            File { url: "https://evil.test/file?token=secret".to_owned(), ..metadata.clone() },
            File { external_type: Some("File".to_owned()), ..metadata },
        ] {
            let error = client.explain_file_for_mcp(&file, McpFileAdmission::try_acquire().unwrap(), async {
                panic!("rejected metadata must not reach the pre-model owner guard")
            }).await.unwrap_err().to_string();
            assert!(!error.contains("secret"));
            assert!(!error.contains("https://"));
            assert!(!error.contains("/tmp/private"));
        }
    }
}
