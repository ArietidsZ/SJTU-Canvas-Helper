//! Build calendar URLs without changing date/offset or repeated-context values.
//! Request construction only: this helper never sends an HTTP request.

pub(super) fn calendar_events_url(
    client: &reqwest::Client,
    base_url: &str,
    context_codes: &[String],
    start_date: &str,
    end_date: &str,
) -> Result<reqwest::Url, reqwest::Error> {
    let endpoint = format!("{}/api/v1/calendar_events", base_url.trim_end_matches('/'));
    let mut query = vec![
        ("type", "assignment"),
        ("start_date", start_date),
        ("end_date", end_date),
    ];
    query.extend(context_codes.iter().map(|code| ("context_codes[]", code.as_str())));
    // Form encoding preserves '+' in positive RFC 3339 offsets instead of
    // letting the server interpret it as a space or a new query parameter.
    Ok(client.get(endpoint).query(&query).build()?.url().clone())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn calendar_query_preserves_offsets_and_date_only_values() {
        let client = reqwest::Client::new();
        for (start, end) in [
            ("2026-11-07T08:00:00+08:00", "2026-11-08T08:00:00+08:00"),
            ("2026-11-07T08:00:00-05:00", "2026-11-08T08:00:00Z"),
            ("2026-11-07", "2026-11-08"),
        ] {
            let url = calendar_events_url(&client, "https://oc.sjtu.edu.cn", &["course_7".into()], start, end).unwrap();
            let query: Vec<_> = url.query_pairs().collect();
            assert_eq!(query.iter().find(|(key, _)| key == "start_date").unwrap().1, start);
            assert_eq!(query.iter().find(|(key, _)| key == "end_date").unwrap().1, end);
            assert_eq!(query.iter().find(|(key, _)| key == "context_codes[]").unwrap().1, "course_7");
            assert_eq!(url.path(), "/api/v1/calendar_events");
            if start.contains('+') { assert!(url.as_str().contains("%2B08%3A00")); }
        }
    }

    #[test]
    fn repeated_contexts_and_reserved_characters_are_values_not_query_fields() {
        let client = reqwest::Client::new();
        let codes = vec!["course_7".into(), "course_8&access_token=not-a-token".into(), "course_甲+乙".into()];
        let url = calendar_events_url(&client, "https://oc.sjtu.edu.cn/", &codes, "2026-11-07&extra=1", "2026-11-08").unwrap();
        let pairs: Vec<_> = url.query_pairs().collect();
        let contexts: Vec<_> = pairs.iter().filter(|(key, _)| key == "context_codes[]").map(|(_, value)| value.as_ref()).collect();
        assert_eq!(contexts, codes.iter().map(String::as_str).collect::<Vec<_>>());
        assert_eq!(pairs.len(), 6);
        assert!(!pairs.iter().any(|(key, _)| key == "access_token" || key == "extra"));
        assert_eq!(pairs.iter().find(|(key, _)| key == "start_date").unwrap().1, "2026-11-07&extra=1");
        assert_eq!(url.path(), "/api/v1/calendar_events");
    }

    #[test]
    fn no_contexts_and_invalid_base_remain_explicit() {
        let client = reqwest::Client::new();
        let url = calendar_events_url(&client, "https://jicanvas.com", &[], "2026-11-07", "2026-11-08").unwrap();
        assert_eq!(url.query_pairs().count(), 3);
        assert!(calendar_events_url(&client, "not a valid URL", &[], "2026-11-07", "2026-11-08").is_err());
    }
}
