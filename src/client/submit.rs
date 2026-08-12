use crate::client::model::Problem;
use crate::client::parser::{
    AtCoderPageError, classify_alert, parse_alert, parse_csrf_token, parse_submission_detail_href,
};
use crate::client::{AtCoderClient, endpoints};
use crate::validation::validate_atcoder_identifier;
use anyhow::{Context, Result, bail};
use reqwest::StatusCode;
use serde::Serialize;

#[derive(Serialize, Debug)]
struct SubmitData {
    #[serde(rename = "data.TaskScreenName")]
    task_name: String,
    #[serde(rename = "data.LanguageId")]
    language_id: String,
    #[serde(rename = "sourceCode")]
    source_code: String,
    #[serde(rename = "csrf_token")]
    csrf_token: String,
}

fn submission_error(error: AtCoderPageError) -> anyhow::Error {
    match error {
        AtCoderPageError::ContestNotFound => {
            anyhow::anyhow!("Failed to submit task: Contest not found.")
        }
        AtCoderPageError::TaskNotFound => {
            anyhow::anyhow!("Failed to submit task: Task not found.")
        }
        AtCoderPageError::PermissionDenied => anyhow::anyhow!(
            r#"Failed to submit task: You are not logged in. Run "ackit login" first."#
        ),
        AtCoderPageError::SourceTooLong => {
            anyhow::anyhow!("Failed to submit task: Source code is too long.")
        }
        AtCoderPageError::SourceEmpty => {
            anyhow::anyhow!("Failed to submit task: Source code is empty.")
        }
        AtCoderPageError::TurnstileRequired => {
            anyhow::anyhow!("Failed to submit task: Error. (Maybe Cloudflare Turnstile Required)")
        }
        AtCoderPageError::UnknownAlert(alert) => {
            anyhow::anyhow!("Failed to submit task: {alert}")
        }
    }
}

fn absolute_url(href: &str) -> String {
    if href.starts_with("http://") || href.starts_with("https://") {
        href.to_string()
    } else if href.starts_with('/') {
        format!("{}{href}", endpoints::BASE)
    } else {
        format!("{}/{href}", endpoints::BASE)
    }
}

impl AtCoderClient {
    pub(crate) async fn submit_solution(
        &self,
        contest_id: &str,
        problem: &Problem,
        language_id: u16,
        source_code: String,
    ) -> Result<String> {
        validate_atcoder_identifier(contest_id, "contest ID")?;
        validate_atcoder_identifier(&problem.id, "problem ID")?;
        let problem_url = endpoints::problem(contest_id, &problem.id);
        let problem_page = self.get_page(&problem_url).await?;
        if problem_page.status == StatusCode::NOT_FOUND {
            let error = parse_alert(&problem_page.body)?
                .map(|alert| classify_alert(&alert))
                .unwrap_or_else(|| AtCoderPageError::UnknownAlert("404".into()));
            return Err(submission_error(error));
        }
        if !problem_page.status.is_success() {
            bail!(
                "Failed to fetch task before submission: status {}.",
                problem_page.status.as_u16()
            );
        }
        let csrf_token = parse_csrf_token(&problem_page.body)?;
        let payload = SubmitData {
            task_name: problem.id.clone(),
            language_id: language_id.to_string(),
            source_code,
            csrf_token,
        };

        let response = self
            .http
            .post(endpoints::submit(contest_id))
            .headers(self.headers())
            .form(&payload)
            .send()
            .await
            .context("Failed to submit task.")?;
        let status = response.status();
        let body = response
            .text()
            .await
            .context("Failed to read submission response.")?;
        if let Some(alert) = parse_alert(&body)? {
            return Err(submission_error(classify_alert(&alert)));
        }
        if status != StatusCode::FOUND {
            bail!("Failed to submit task: status {}", status.as_u16());
        }

        let submissions = self.get_page(&endpoints::submissions(contest_id)).await?;
        if !submissions.status.is_success() {
            bail!(
                "Failed to get submissions: status {}.",
                submissions.status.as_u16()
            );
        }
        Ok(absolute_url(&parse_submission_detail_href(
            &submissions.body,
        )?))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn resolves_submission_urls() {
        assert_eq!(
            absolute_url("/contests/abc999/submissions/1"),
            "https://atcoder.jp/contests/abc999/submissions/1"
        );
        assert_eq!(
            absolute_url("https://example.com/1"),
            "https://example.com/1"
        );
    }
}
