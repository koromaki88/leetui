use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TopicTag {
    pub name: String,
    pub slug: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ProblemSummary {
    pub ac_rate: f64,
    pub difficulty: String,
    pub frontend_question_id: String,
    pub paid_only: bool,
    pub status: Option<String>,
    pub title: String,
    pub title_slug: String,
    #[serde(default)]
    pub topic_tags: Vec<TopicTag>,
    #[serde(default)]
    pub has_solution: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CodeSnippet {
    pub lang: String,
    pub lang_slug: String,
    pub code: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SolutionAccess {
    pub id: String,
    pub can_see_detail: bool,
    pub paid_only: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Problem {
    pub question_id: String,
    pub question_frontend_id: String,
    pub title: String,
    pub title_slug: String,
    pub content: String,
    pub difficulty: String,
    pub is_paid_only: bool,
    pub status: Option<String>,
    pub sample_test_case: String,
    pub example_testcases: Option<String>,
    pub meta_data: String,
    #[serde(default)]
    pub code_snippets: Vec<CodeSnippet>,
    #[serde(default)]
    pub topic_tags: Vec<TopicTag>,
    #[serde(default)]
    pub hints: Vec<String>,
    pub stats: Option<String>,
    pub solution: Option<SolutionAccess>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Editorial {
    pub id: String,
    pub title: String,
    pub content: String,
    pub paid_only: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SubmissionSummary {
    pub id: String,
    pub status_display: String,
    pub lang: String,
    pub runtime: Option<String>,
    pub timestamp: String,
    pub url: Option<String>,
    pub memory: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SubmissionDetails {
    pub id: Option<String>,
    pub code: String,
    pub status_display: Option<String>,
    pub runtime_display: Option<String>,
    pub memory_display: Option<String>,
    pub runtime_error: Option<String>,
    pub compile_error: Option<String>,
}

#[derive(Debug, Clone)]
pub struct JudgeResult {
    pub status: String,
    pub runtime: Option<String>,
    pub memory: Option<String>,
    pub input: Option<String>,
    pub output: Option<String>,
    pub expected: Option<String>,
    pub error: Option<String>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_representative_problem_fixture() {
        let problem: Problem = serde_json::from_value(serde_json::json!({
            "questionId": "1",
            "questionFrontendId": "1",
            "title": "Two Sum",
            "titleSlug": "two-sum",
            "content": "<p>Statement</p>",
            "difficulty": "Easy",
            "isPaidOnly": false,
            "status": "ac",
            "sampleTestCase": "[2,7]\n9",
            "exampleTestcases": "[2,7]\n9",
            "metaData": "{}",
            "codeSnippets": [{"lang": "Rust", "langSlug": "rust", "code": "impl Solution {}"}],
            "topicTags": [{"name": "Array", "slug": "array"}],
            "hints": [],
            "stats": "{}",
            "solution": {"id": "7", "canSeeDetail": true, "paidOnly": false}
        }))
        .unwrap();
        assert_eq!(problem.title_slug, "two-sum");
        assert_eq!(problem.code_snippets[0].lang_slug, "rust");
    }

    #[test]
    fn parses_representative_submission_fixture() {
        let submission: SubmissionSummary = serde_json::from_value(serde_json::json!({
            "id": "123",
            "statusDisplay": "Accepted",
            "lang": "rust",
            "runtime": "0 ms",
            "timestamp": "1700000000",
            "url": "/submissions/detail/123/",
            "isPending": "Not Pending",
            "memory": "2 MB"
        }))
        .unwrap();
        assert_eq!(submission.status_display, "Accepted");
    }
}
