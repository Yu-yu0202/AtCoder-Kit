use crate::client::model::{Contest, ContestType, Problem};
use crate::client::parser::{
    AtCoderPageError, TaskRef, classify_alert, parse_alert, parse_contest_title,
    parse_problem_page, parse_task_refs,
};
use crate::client::{AtCoderClient, endpoints};
use crate::validation::validate_atcoder_identifier;
use anyhow::{Context, Result, bail};
use reqwest::StatusCode;
use std::collections::BTreeMap;

fn page_error(context: &str, body: &str) -> anyhow::Error {
    let error = parse_alert(body)
        .ok()
        .flatten()
        .map(|alert| classify_alert(&alert));
    match error {
        Some(AtCoderPageError::ContestNotFound) => {
            anyhow::anyhow!("Failed to {context}: Contest not found.")
        }
        Some(AtCoderPageError::TaskNotFound) => {
            anyhow::anyhow!("Failed to {context}: Task not found.")
        }
        Some(AtCoderPageError::PermissionDenied) => anyhow::anyhow!(
            r#"Failed to {context}: You are not logged in. Run "ackit login" first."#
        ),
        Some(AtCoderPageError::UnknownAlert(alert)) => {
            anyhow::anyhow!("Failed to {context}: {alert}")
        }
        _ => anyhow::anyhow!("Failed to {context}: Unexpected response."),
    }
}

impl AtCoderClient {
    async fn fetch_problem(&self, contest_id: &str, problem_id: &str) -> Result<Problem> {
        validate_atcoder_identifier(contest_id, "contest ID")?;
        validate_atcoder_identifier(problem_id, "problem ID")?;
        let url = endpoints::problem(contest_id, problem_id);
        let page = self.get_page(&url).await?;
        if page.status == StatusCode::NOT_FOUND {
            return Err(page_error("fetch task", &page.body));
        }
        if !page.status.is_success() {
            bail!("Failed to fetch task: status {}.", page.status.as_u16());
        }

        let parsed = parse_problem_page(&page.body).context("Failed to parse problem page.")?;
        Ok(Problem {
            id: problem_id.to_string(),
            contest_id: contest_id.to_string(),
            label: parsed.label,
            url,
            title: parsed.title,
            memory_limit_bytes: parsed.memory_limit_bytes,
            time_limit_msecs: parsed.time_limit_msecs,
            sample_cases: parsed.sample_cases,
        })
    }

    async fn fetch_task_refs(&self, contest_id: &str) -> Result<Vec<TaskRef>> {
        let page = self.get_page(&endpoints::tasks(contest_id)).await?;
        if page.status == StatusCode::NOT_FOUND {
            return Err(page_error("fetch tasks", &page.body));
        }
        if !page.status.is_success() {
            bail!("Failed to fetch tasks: status {}.", page.status.as_u16());
        }

        parse_task_refs(&page.body, contest_id)
    }

    pub(crate) async fn fetch_problem_by_label(
        &self,
        contest_id: &str,
        label: &str,
    ) -> Result<Problem> {
        validate_atcoder_identifier(contest_id, "contest ID")?;
        let task = self
            .fetch_task_refs(contest_id)
            .await?
            .into_iter()
            .find(|task| task.label.eq_ignore_ascii_case(label))
            .with_context(|| {
                format!("Problem label '{label}' not found in contest '{contest_id}'.")
            })?;
        self.fetch_problem(contest_id, &task.id).await
    }

    async fn fetch_problems(&self, contest_id: &str) -> Result<BTreeMap<String, Problem>> {
        let tasks = self.fetch_task_refs(contest_id).await?;
        let mut problems = BTreeMap::new();
        for task in tasks {
            let problem = self.fetch_problem(contest_id, &task.id).await?;
            let label = problem.label.clone();
            if problems.insert(label.clone(), problem).is_some() {
                bail!("Duplicate problem label: {label}");
            }
        }
        Ok(problems)
    }

    pub(crate) async fn fetch_contest(&self, contest_id: &str) -> Result<Contest> {
        validate_atcoder_identifier(contest_id, "contest ID")?;
        let page = self.get_page(&endpoints::contest(contest_id)).await?;
        if page.status == StatusCode::NOT_FOUND {
            return Err(page_error("fetch contest", &page.body));
        }
        if !page.status.is_success() {
            bail!("Failed to fetch contest: status {}.", page.status.as_u16());
        }

        let title = parse_contest_title(&page.body)?;
        let problems = self.fetch_problems(contest_id).await?;
        Ok(Contest {
            id: contest_id.to_string(),
            contest_type: ContestType::from_contest_id(contest_id),
            title,
            problems,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use reqwest_middleware::{ClientBuilder, Middleware, Next};

    struct FixturePages {
        tasks_only: bool,
    }

    #[async_trait::async_trait]
    impl Middleware for FixturePages {
        async fn handle(
            &self,
            request: reqwest::Request,
            _extensions: &mut http::Extensions,
            _next: Next<'_>,
        ) -> reqwest_middleware::Result<reqwest::Response> {
            let body = match request.url().path() {
                "/contests/abc001/tasks" => {
                    include_str!("../../tests/fixtures/atcoder/tasks_abc001.html")
                }
                "/contests/abc001/tasks/abc001_1" if !self.tasks_only => {
                    include_str!("../../tests/fixtures/atcoder/problem_legacy.html")
                }
                path => panic!("Unexpected request: {path}"),
            };
            Ok(http::Response::new(body).into())
        }
    }

    fn client(tasks_only: bool) -> AtCoderClient {
        AtCoderClient {
            http: ClientBuilder::new(reqwest::Client::new())
                .with(FixturePages { tasks_only })
                .build(),
            headers: Default::default(),
        }
    }

    #[tokio::test]
    async fn fetches_only_the_linked_problem_for_either_label_case() {
        let client = client(false);
        for label in ["A", "a"] {
            let problem = client
                .fetch_problem_by_label("abc001", label)
                .await
                .unwrap();
            assert_eq!(problem.id, "abc001_1");
            assert_eq!(problem.contest_id, "abc001");
            assert_eq!(problem.label, "A");
            assert_eq!(
                problem.url,
                "https://atcoder.jp/contests/abc001/tasks/abc001_1"
            );
            assert_eq!(problem.sample_cases.len(), 1);
        }
    }

    #[tokio::test]
    async fn reports_missing_label_without_fetching_a_problem_page() {
        let error = client(true)
            .fetch_problem_by_label("abc001", "Z")
            .await
            .unwrap_err();

        assert_eq!(
            error.to_string(),
            "Problem label 'Z' not found in contest 'abc001'."
        );
    }
}
