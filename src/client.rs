use std::time::{Duration, Instant};

use reqwest::{Client, Method, Response, StatusCode, Url, header, redirect};
use serde::{Deserialize, Serialize, de::DeserializeOwned};
use serde_json::{Value, json};
use thiserror::Error;
use tokio::time::sleep;

use crate::{
    config::Credentials,
    model::{
        Editorial, JudgeResult, Problem, ProblemSummary, SubmissionDetails, SubmissionSummary,
    },
};

const BASE: &str = "https://leetcode.com";

#[derive(Debug, Error)]
pub enum ApiError {
    #[error("authentication is required; run `leetui login`")]
    AuthenticationRequired,
    #[error("the LeetCode session has expired; run `leetui login` again")]
    SessionExpired,
    #[error("LeetCode rate-limited the request; wait and try again")]
    RateLimited,
    #[error("LeetCode returned HTTP {status} for {url}{detail}")]
    HttpStatus {
        status: StatusCode,
        url: String,
        detail: String,
    },
    #[error("LeetCode API error: {0}")]
    GraphQl(String),
    #[error("unexpected LeetCode response: {0}")]
    InvalidResponse(String),
    #[error("judge result timed out")]
    JudgeTimeout,
    #[error(transparent)]
    Network(#[from] reqwest::Error),
}

pub type ApiResult<T> = Result<T, ApiError>;

#[derive(Clone)]
pub struct LeetCodeClient {
    http: Client,
    credentials: Option<Credentials>,
}

#[derive(Debug, Deserialize)]
struct GraphQlEnvelope<T> {
    data: Option<T>,
    #[serde(default)]
    errors: Vec<GraphQlError>,
}

#[derive(Debug, Deserialize)]
struct GraphQlError {
    message: String,
}

impl LeetCodeClient {
    pub fn new(credentials: Option<Credentials>) -> ApiResult<Self> {
        let http = Client::builder()
            .user_agent(concat!("leetui/", env!("CARGO_PKG_VERSION")))
            .timeout(Duration::from_secs(30))
            .redirect(redirect::Policy::custom(|attempt| {
                if attempt.previous().len() >= 5 {
                    attempt.error("too many redirects")
                } else if allowed_remote_url(attempt.url()) {
                    attempt.follow()
                } else {
                    attempt.stop()
                }
            }))
            .build()?;
        Ok(Self { http, credentials })
    }

    pub async fn validate_session(&self) -> ApiResult<String> {
        #[derive(Deserialize)]
        struct Data {
            #[serde(rename = "userStatus")]
            user_status: UserStatus,
        }
        #[derive(Deserialize)]
        #[serde(rename_all = "camelCase")]
        struct UserStatus {
            is_signed_in: bool,
            username: Option<String>,
        }

        let data: Data = self
            .graphql(
                "globalData",
                "query globalData { userStatus { isSignedIn username } }",
                json!({}),
                None,
            )
            .await?;
        if !data.user_status.is_signed_in {
            return Err(ApiError::SessionExpired);
        }
        data.user_status
            .username
            .ok_or_else(|| ApiError::InvalidResponse("signed-in response had no username".into()))
    }

    pub async fn problem_list(&self) -> ApiResult<Vec<ProblemSummary>> {
        #[derive(Deserialize)]
        #[serde(rename_all = "camelCase")]
        struct Page {
            total: usize,
            questions: Vec<ProblemSummary>,
        }
        #[derive(Deserialize)]
        #[serde(rename_all = "camelCase")]
        struct Data {
            problemset_question_list: Page,
        }

        const QUERY: &str = r#"
query problemsetQuestionList($categorySlug: String, $limit: Int, $skip: Int, $filters: QuestionListFilterInput) {
  problemsetQuestionList: questionList(categorySlug: $categorySlug, limit: $limit, skip: $skip, filters: $filters) {
    total: totalNum
    questions: data {
      acRate difficulty frontendQuestionId: questionFrontendId paidOnly: isPaidOnly
      status title titleSlug topicTags { name slug } hasSolution
    }
  }
}"#;

        let mut problems = Vec::new();
        loop {
            let data: Data = self.graphql(
                "problemsetQuestionList",
                QUERY,
                json!({"categorySlug": "", "skip": problems.len(), "limit": 100, "filters": {}}),
                None,
            ).await?;
            let total = data.problemset_question_list.total;
            if data.problemset_question_list.questions.is_empty() {
                break;
            }
            problems.extend(data.problemset_question_list.questions);
            if problems.len() >= total {
                break;
            }
            sleep(Duration::from_millis(75)).await;
        }
        Ok(problems)
    }

    pub async fn problem(&self, slug: &str) -> ApiResult<Problem> {
        #[derive(Deserialize)]
        struct Data {
            question: Option<Problem>,
        }
        const QUERY: &str = r#"
query questionData($titleSlug: String!) {
  question(titleSlug: $titleSlug) {
    questionId questionFrontendId title titleSlug content difficulty isPaidOnly status
    sampleTestCase exampleTestcases metaData
    codeSnippets { lang langSlug code }
    topicTags { name slug }
    hints stats
    solution { id canSeeDetail paidOnly }
  }
}"#;
        let data: Data = self
            .graphql(
                "questionData",
                QUERY,
                json!({"titleSlug": slug}),
                Some(slug),
            )
            .await?;
        data.question
            .ok_or_else(|| ApiError::InvalidResponse(format!("problem `{slug}` was not found")))
    }

    pub async fn editorial(&self, slug: &str) -> ApiResult<Editorial> {
        #[derive(Deserialize)]
        struct Question {
            solution: Option<Editorial>,
        }
        #[derive(Deserialize)]
        struct Data {
            question: Option<Question>,
        }
        const QUERY: &str = r#"
query officialSolution($titleSlug: String!) {
  question(titleSlug: $titleSlug) {
    solution { id title content paidOnly }
  }
}"#;
        let data: Data = self
            .graphql(
                "officialSolution",
                QUERY,
                json!({"titleSlug": slug}),
                Some(slug),
            )
            .await?;
        data.question.and_then(|q| q.solution).ok_or_else(|| {
            ApiError::InvalidResponse("no accessible official editorial is available".into())
        })
    }

    pub async fn submissions(&self, slug: &str) -> ApiResult<Vec<SubmissionSummary>> {
        self.require_auth()?;
        #[derive(Deserialize)]
        #[serde(rename_all = "camelCase")]
        struct List {
            #[serde(rename = "hasNext")]
            has_next: bool,
            #[serde(rename = "lastKey")]
            last_key: Option<String>,
            submissions: Vec<SubmissionSummary>,
        }
        #[derive(Deserialize)]
        #[serde(rename_all = "camelCase")]
        struct Data {
            submission_list: Option<List>,
        }
        const QUERY: &str = r#"
query Submissions($offset: Int!, $limit: Int!, $lastKey: String, $questionSlug: String!) {
  submissionList(offset: $offset, limit: $limit, lastKey: $lastKey, questionSlug: $questionSlug) {
    hasNext lastKey
    submissions { id statusDisplay lang runtime timestamp url isPending memory }
  }
}"#;
        let mut submissions = Vec::new();
        let mut last_key: Option<String> = None;
        loop {
            let previous_key = last_key.clone();
            let data: Data = self
                .graphql(
                    "Submissions",
                    QUERY,
                    json!({
                        "offset": submissions.len(), "limit": 50,
                        "lastKey": last_key, "questionSlug": slug
                    }),
                    Some(slug),
                )
                .await?;
            let page = data.submission_list.ok_or(ApiError::SessionExpired)?;
            let has_next = page.has_next;
            last_key = page.last_key;
            if page.submissions.is_empty() {
                break;
            }
            submissions.extend(page.submissions);
            if !has_next {
                break;
            }
            if last_key.is_none() || last_key == previous_key {
                return Err(ApiError::InvalidResponse(
                    "submission pagination did not advance".into(),
                ));
            }
            sleep(Duration::from_millis(75)).await;
        }
        Ok(submissions)
    }

    pub async fn submission_details(&self, id: &str) -> ApiResult<SubmissionDetails> {
        self.require_auth()?;
        #[derive(Deserialize)]
        #[serde(rename_all = "camelCase")]
        struct Data {
            submission_details: Option<SubmissionDetails>,
        }
        const QUERY: &str = r#"
query submissionDetails($submissionId: Int!) {
  submissionDetails(submissionId: $submissionId) {
    id code statusDisplay runtimeDisplay memoryDisplay runtimeError compileError
  }
}"#;
        let id: i64 = id
            .parse()
            .map_err(|_| ApiError::InvalidResponse("invalid submission ID".into()))?;
        let data: Data = self
            .graphql(
                "submissionDetails",
                QUERY,
                json!({"submissionId": id}),
                None,
            )
            .await?;
        data.submission_details.ok_or(ApiError::SessionExpired)
    }

    pub async fn download_image(&self, url: &str) -> ApiResult<Vec<u8>> {
        let url = normalize_image_url(url)?;
        let response = self.http.get(url).send().await?;
        let mut response = self.check_status(response).await?;
        const MAX_IMAGE_BYTES: usize = 10 * 1024 * 1024;
        if response.url().scheme() != "https" {
            return Err(ApiError::InvalidResponse(
                "refusing an image redirect to a non-HTTPS URL".into(),
            ));
        }
        if let Some(content_type) = response.headers().get(header::CONTENT_TYPE)
            && !content_type
                .to_str()
                .unwrap_or_default()
                .to_ascii_lowercase()
                .starts_with("image/")
        {
            return Err(ApiError::InvalidResponse(
                "image URL returned a non-image content type".into(),
            ));
        }
        if response
            .content_length()
            .is_some_and(|length| length > MAX_IMAGE_BYTES as u64)
        {
            return Err(ApiError::InvalidResponse(
                "image exceeds the 10 MiB limit".into(),
            ));
        }
        let mut bytes = Vec::with_capacity(
            response
                .content_length()
                .unwrap_or(0)
                .min(MAX_IMAGE_BYTES as u64) as usize,
        );
        while let Some(chunk) = response.chunk().await? {
            append_limited(&mut bytes, &chunk, MAX_IMAGE_BYTES)?;
        }
        Ok(bytes)
    }

    pub async fn run(
        &self,
        problem: &Problem,
        lang: &str,
        code: &str,
        input: &str,
    ) -> ApiResult<JudgeResult> {
        let id = self
            .judge_request(
                &format!("/problems/{}/interpret_solution/", problem.title_slug),
                &problem.title_slug,
                json!({
                    "lang": lang,
                    "question_id": problem.question_id,
                    "typed_code": code,
                    "data_input": input,
                }),
                "interpret_id",
            )
            .await?;
        self.poll_judge(&id).await
    }

    pub async fn submit(
        &self,
        problem: &Problem,
        lang: &str,
        code: &str,
    ) -> ApiResult<JudgeResult> {
        let id = self
            .judge_request(
                &format!("/problems/{}/submit/", problem.title_slug),
                &problem.title_slug,
                json!({
                    "lang": lang,
                    "question_id": problem.question_id,
                    "typed_code": code,
                }),
                "submission_id",
            )
            .await?;
        self.poll_judge(&id).await
    }

    async fn judge_request(
        &self,
        path: &str,
        slug: &str,
        body: Value,
        id_field: &str,
    ) -> ApiResult<String> {
        self.require_auth()?;
        let response = self
            .request(Method::POST, &format!("{BASE}{path}"), Some(slug))
            .json(&body)
            .send()
            .await?;
        let response = self.check_status(response).await?;
        let value: Value = response.json().await?;
        value
            .get(id_field)
            .and_then(value_to_string)
            .ok_or_else(|| ApiError::InvalidResponse(format!("judge response had no {id_field}")))
    }

    async fn poll_judge(&self, id: &str) -> ApiResult<JudgeResult> {
        let started = Instant::now();
        let mut delay = Duration::from_millis(250);
        loop {
            if started.elapsed() > Duration::from_secs(120) {
                return Err(ApiError::JudgeTimeout);
            }
            let response = self
                .request(
                    Method::GET,
                    &format!("{BASE}/submissions/detail/{id}/check/"),
                    None,
                )
                .send()
                .await?;
            let response = self.check_status(response).await?;
            let value: Value = response.json().await?;
            match value.get("state").and_then(Value::as_str).unwrap_or("") {
                "SUCCESS" => return Ok(parse_judge_result(&value)),
                "FAILURE" => {
                    return Err(ApiError::InvalidResponse(
                        "judge failed to process the request".into(),
                    ));
                }
                _ => {
                    sleep(delay).await;
                    delay = (delay * 2).min(Duration::from_secs(2));
                }
            }
        }
    }

    async fn graphql<T: DeserializeOwned>(
        &self,
        operation: &str,
        query: &str,
        variables: Value,
        slug: Option<&str>,
    ) -> ApiResult<T> {
        #[derive(Serialize)]
        #[serde(rename_all = "camelCase")]
        struct Body<'a> {
            operation_name: &'a str,
            query: &'a str,
            variables: Value,
        }
        let response = self
            .request(Method::POST, &format!("{BASE}/graphql/"), slug)
            .json(&Body {
                operation_name: operation,
                query,
                variables,
            })
            .send()
            .await?;
        let response = self.check_status(response).await?;
        let envelope: GraphQlEnvelope<T> = response.json().await?;
        if !envelope.errors.is_empty() {
            return Err(ApiError::GraphQl(format!(
                "operation `{operation}`: {}",
                envelope
                    .errors
                    .into_iter()
                    .map(|e| e.message)
                    .collect::<Vec<_>>()
                    .join("; ")
            )));
        }
        envelope
            .data
            .ok_or_else(|| ApiError::InvalidResponse("GraphQL response had no data".into()))
    }

    fn request(&self, method: Method, url: &str, slug: Option<&str>) -> reqwest::RequestBuilder {
        let referer = slug
            .map(|s| format!("{BASE}/problems/{s}/"))
            .unwrap_or_else(|| BASE.into());
        let mut request = self
            .http
            .request(method, url)
            .header(header::ORIGIN, BASE)
            .header(header::REFERER, referer)
            .header(header::ACCEPT, "application/json")
            .header(header::CONTENT_TYPE, "application/json");
        if let Some(credentials) = &self.credentials {
            request = request
                .header(
                    header::COOKIE,
                    format!(
                        "LEETCODE_SESSION={}; csrftoken={}",
                        credentials.session, credentials.csrf
                    ),
                )
                .header("x-csrftoken", &credentials.csrf);
        }
        request
    }

    async fn check_status(&self, mut response: Response) -> ApiResult<Response> {
        match response.status() {
            StatusCode::UNAUTHORIZED => Err(ApiError::SessionExpired),
            StatusCode::TOO_MANY_REQUESTS => Err(ApiError::RateLimited),
            status if !status.is_success() => {
                let url = response.url().to_string();
                let mut body = Vec::new();
                const MAX_ERROR_BODY: usize = 4096;
                while let Some(chunk) = response.chunk().await? {
                    let remaining = MAX_ERROR_BODY.saturating_sub(body.len());
                    if remaining == 0 {
                        break;
                    }
                    body.extend_from_slice(&chunk[..chunk.len().min(remaining)]);
                }
                let excerpt = String::from_utf8_lossy(&body);
                let sanitized = excerpt.split_whitespace().collect::<Vec<_>>().join(" ");
                let detail = if sanitized.is_empty() {
                    String::new()
                } else {
                    format!(": {sanitized}")
                };
                Err(ApiError::HttpStatus {
                    status,
                    url,
                    detail,
                })
            }
            _ => Ok(response),
        }
    }

    fn require_auth(&self) -> ApiResult<&Credentials> {
        self.credentials
            .as_ref()
            .ok_or(ApiError::AuthenticationRequired)
    }
}

fn normalize_image_url(url: &str) -> ApiResult<String> {
    let normalized = if let Some(rest) = url.strip_prefix("//") {
        format!("https://{rest}")
    } else if url.starts_with('/') {
        format!("{BASE}{url}")
    } else {
        url.to_owned()
    };
    let parsed = Url::parse(&normalized)
        .map_err(|_| ApiError::InvalidResponse("image URL is invalid".into()))?;
    if !allowed_remote_url(&parsed) {
        return Err(ApiError::InvalidResponse(
            "image URL is not on an allowed LeetCode asset host".into(),
        ));
    }
    Ok(parsed.into())
}

fn allowed_remote_url(url: &Url) -> bool {
    if url.scheme() != "https" {
        return false;
    }
    let Some(host) = url.host_str() else {
        return false;
    };
    host == "leetcode.com"
        || host.ends_with(".leetcode.com")
        || host == "s3-lc-upload.s3.amazonaws.com"
}

fn append_limited(buffer: &mut Vec<u8>, chunk: &[u8], limit: usize) -> ApiResult<()> {
    let next_len = buffer
        .len()
        .checked_add(chunk.len())
        .ok_or_else(|| ApiError::InvalidResponse("image size overflowed address space".into()))?;
    if next_len > limit {
        return Err(ApiError::InvalidResponse(
            "image exceeds the 10 MiB limit".into(),
        ));
    }
    buffer.extend_from_slice(chunk);
    Ok(())
}

fn value_to_string(value: &Value) -> Option<String> {
    value
        .as_str()
        .map(str::to_owned)
        .or_else(|| value.as_i64().map(|v| v.to_string()))
}

fn display_value(value: Option<&Value>) -> Option<String> {
    match value? {
        Value::String(v) => Some(v.clone()),
        Value::Array(values) => Some(
            values
                .iter()
                .filter_map(|v| v.as_str())
                .collect::<Vec<_>>()
                .join("\n"),
        ),
        Value::Null => None,
        other => Some(other.to_string()),
    }
}

fn parse_judge_result(value: &Value) -> JudgeResult {
    let error = [
        "full_compile_error",
        "compile_error",
        "full_runtime_error",
        "runtime_error",
    ]
    .into_iter()
    .find_map(|key| display_value(value.get(key)));
    JudgeResult {
        status: value
            .get("status_msg")
            .and_then(Value::as_str)
            .unwrap_or("Unknown")
            .to_owned(),
        runtime: display_value(value.get("status_runtime")),
        memory: display_value(value.get("status_memory")),
        input: display_value(value.get("input")),
        output: display_value(value.get("code_output")),
        expected: display_value(value.get("expected_code_answer")),
        error,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn image_urls_must_use_https() {
        assert_eq!(
            normalize_image_url("//assets.leetcode.com/a.png").unwrap(),
            "https://assets.leetcode.com/a.png"
        );
        assert!(normalize_image_url("http://example.com/a.png").is_err());
        assert!(normalize_image_url("https://example.com/a.png").is_err());
    }

    #[test]
    fn parses_array_judge_output() {
        let result = parse_judge_result(&json!({
            "status_msg": "Accepted",
            "status_runtime": "1 ms",
            "code_output": ["[0,1]"],
            "expected_code_answer": ["[0,1]"]
        }));
        assert_eq!(result.status, "Accepted");
        assert_eq!(result.output.as_deref(), Some("[0,1]"));
    }

    #[test]
    fn streaming_limit_rejects_before_appending_oversized_chunk() {
        let mut bytes = vec![0; 6];
        assert!(append_limited(&mut bytes, &[1; 5], 10).is_err());
        assert_eq!(bytes.len(), 6);
        append_limited(&mut bytes, &[1; 4], 10).unwrap();
        assert_eq!(bytes.len(), 10);
    }
}
