use crate::client::model::{Contest, ContestType, Problem};
use crate::client::parser::{
    AtCoderPageError, classify_alert, parse_alert, parse_contest_title, parse_problem_page,
    parse_task_refs,
};
use crate::client::{AtCoderClient, endpoints};
use crate::validation::validate_atcoder_identifier;
use anyhow::{Context, Result, bail};
use reqwest::StatusCode;
use std::collections::HashMap;

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

    async fn fetch_problems(&self, contest_id: &str) -> Result<HashMap<String, Problem>> {
        let page = self.get_page(&endpoints::tasks(contest_id)).await?;
        if page.status == StatusCode::NOT_FOUND {
            return Err(page_error("fetch tasks", &page.body));
        }
        if !page.status.is_success() {
            bail!("Failed to fetch tasks: status {}.", page.status.as_u16());
        }

        let tasks = parse_task_refs(&page.body, contest_id)?;
        let mut problems = HashMap::with_capacity(tasks.len());
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
