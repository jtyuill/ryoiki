use reqwest::StatusCode;
use serde::{Deserialize, Serialize};
use thiserror::Error;

const KANA_VN_ENDPOINT: &str = "https://api.vndb.org/kana/vn";
const RESULT_LIMIT: u8 = 10;

#[derive(Clone, Debug)]
pub struct VndbClient {
    http: reqwest::Client,
    endpoint: String,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq)]
pub struct VnSummary {
    pub id: String,
    pub title: String,
    pub alttitle: Option<String>,
    pub released: Option<String>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct VndbSearchResults {
    pub entries: Vec<VnSummary>,
    pub more: bool,
}

#[derive(Debug, Error)]
pub enum VndbError {
    #[error("enter a title to search VNDB")]
    EmptyQuery,
    #[error("VNDB request failed: {0}")]
    Request(#[from] reqwest::Error),
    #[error("VNDB returned HTTP {status}: {message}")]
    Api { status: StatusCode, message: String },
}

#[derive(Debug, Serialize)]
struct SearchRequest<'query> {
    filters: [&'query str; 3],
    fields: &'static str,
    sort: &'static str,
    results: u8,
}

#[derive(Debug, Deserialize)]
struct SearchResponse {
    results: Vec<VnSummary>,
    more: bool,
}

#[derive(Debug, Deserialize)]
struct ApiErrorResponse {
    id: Option<String>,
    msg: Option<String>,
}

impl VndbClient {
    pub fn new() -> Self {
        Self {
            http: reqwest::Client::new(),
            endpoint: KANA_VN_ENDPOINT.to_owned(),
        }
    }

    #[cfg(test)]
    fn with_endpoint(endpoint: String) -> Self {
        Self {
            http: reqwest::Client::new(),
            endpoint,
        }
    }

    pub async fn search(&self, query: &str) -> Result<VndbSearchResults, VndbError> {
        let query = query.trim();
        if query.is_empty() {
            return Err(VndbError::EmptyQuery);
        }

        let request = SearchRequest {
            filters: ["search", "=", query],
            fields: "title,alttitle,released",
            sort: "searchrank",
            results: RESULT_LIMIT,
        };

        let response = self
            .http
            .post(&self.endpoint)
            .header(
                reqwest::header::USER_AGENT,
                concat!("ryoiki/", env!("CARGO_PKG_VERSION")),
            )
            .json(&request)
            .send()
            .await?;

        let status = response.status();
        if !status.is_success() {
            let body = response.text().await?;
            return Err(VndbError::Api {
                status,
                message: api_error_message(&body),
            });
        }

        let response: SearchResponse = response.json().await?;
        Ok(VndbSearchResults {
            entries: response.results,
            more: response.more,
        })
    }
}

fn api_error_message(body: &str) -> String {
    if let Ok(error) = serde_json::from_str::<ApiErrorResponse>(body) {
        match (error.id, error.msg) {
            (Some(id), Some(message)) => return format!("{id}: {message}"),
            (_, Some(message)) => return message,
            (Some(id), None) => return id,
            (None, None) => {}
        }
    }

    let excerpt: String = body.chars().take(300).collect();
    if excerpt.is_empty() {
        "empty error response".to_owned()
    } else {
        excerpt
    }
}

#[cfg(test)]
mod tests {
    use httpmock::{Method::POST, MockServer};
    use serde_json::json;

    use super::{VndbClient, VndbError};

    #[tokio::test]
    async fn searches_kana_and_decodes_results() {
        let server = MockServer::start();
        let search = server.mock(|when, then| {
            when.method(POST).path("/vn").json_body(json!({
                "filters": ["search", "=", "Subarashiki Hibi"],
                "fields": "title,alttitle,released",
                "sort": "searchrank",
                "results": 10
            }));
            then.status(200).json_body(json!({
                "results": [{
                    "id": "v3144",
                    "title": "Subarashiki Hibi ~Furenzoku Sonzai~",
                    "alttitle": "素晴らしき日々 ～不連続存在～",
                    "released": "2010-03-26"
                }],
                "more": false
            }));
        });
        let client = VndbClient::with_endpoint(server.url("/vn"));

        let results = client
            .search("  Subarashiki Hibi  ")
            .await
            .expect("search response should decode");

        search.assert();
        assert_eq!(results.entries.len(), 1);
        assert_eq!(results.entries[0].id, "v3144");
        assert_eq!(results.entries[0].released.as_deref(), Some("2010-03-26"));
        assert!(!results.more);
    }

    #[tokio::test]
    async fn rejects_empty_queries_without_a_request() {
        let client = VndbClient::new();

        let error = client
            .search("  ")
            .await
            .expect_err("empty query should fail");

        assert!(matches!(error, VndbError::EmptyQuery));
    }
}
