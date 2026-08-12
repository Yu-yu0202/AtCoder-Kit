pub(crate) mod sample;
mod submit;

use crate::application::sample::{TestResult, run_sample_tests};
use crate::application::submit::{prepare_solution, submit_prepared_solution};
use crate::client::AtCoderClient;
use crate::client::auth::{prompt_revel_session, verify_current_session};
use crate::client::cookie::Cookie;
use crate::validation::validate_atcoder_identifier;
use crate::workspace::command::SystemCommandRunner;
use crate::workspace::contest::save_contest_to;
use crate::workspace::problem::ProblemWorkspace;
use crate::workspace::template::{NewTemplate, TemplateRegistry, create_template};
use anyhow::{Context, Result};
use std::path::PathBuf;

pub(crate) enum AppEvent {
    FetchingContest(String),
    SavingContest(String),
    Testing,
    TestSuccessful,
    Submitting,
    SubmitSuccessful,
    LoggingIn,
    LoggingOut,
}

pub(crate) enum LoginOutcome {
    LoggedIn { username: String },
    AlreadyLoggedIn { username: String },
    ExistingSessionInvalid,
}

pub(crate) enum SessionStatus {
    LoggedIn { username: String },
    Invalid,
    LoggedOut,
}

pub(crate) struct DownloadOutcome {
    pub(crate) path: PathBuf,
}

pub(crate) struct SubmitOutcome {
    pub(crate) submission_url: String,
}

pub(crate) struct TemplateCreated {
    pub(crate) path: PathBuf,
}

pub(crate) struct Application {
    start_path: PathBuf,
}

impl Application {
    pub(crate) fn new(start_path: PathBuf) -> Self {
        Self { start_path }
    }

    pub(crate) async fn download<F>(
        &self,
        contest_id: &str,
        template_name: Option<&str>,
        no_template: bool,
        mut event: F,
    ) -> Result<DownloadOutcome>
    where
        F: FnMut(AppEvent),
    {
        validate_atcoder_identifier(contest_id, "contest ID")?;
        event(AppEvent::FetchingContest(contest_id.to_string()));
        let client = AtCoderClient::from_stored_session()?;
        let contest = client.fetch_contest(contest_id).await?;

        event(AppEvent::SavingContest(contest_id.to_string()));
        let registry = (!no_template).then(TemplateRegistry::load).transpose()?;
        let template = registry
            .as_ref()
            .map(|registry| registry.select(template_name))
            .transpose()?
            .flatten();
        let path = save_contest_to(&self.start_path, &contest, template)?;
        Ok(DownloadOutcome { path })
    }

    pub(crate) async fn test(&self) -> Result<Vec<TestResult>> {
        let workspace = ProblemWorkspace::discover_from(&self.start_path)?;
        run_sample_tests(&workspace, &SystemCommandRunner).await
    }

    pub(crate) async fn submit<F>(&self, no_test: bool, mut event: F) -> Result<SubmitOutcome>
    where
        F: FnMut(AppEvent),
    {
        event(AppEvent::Testing);
        let workspace = ProblemWorkspace::discover_from(&self.start_path)?;
        let runner = SystemCommandRunner;
        let source = prepare_solution(&workspace, &runner, no_test).await?;
        event(AppEvent::TestSuccessful);

        event(AppEvent::Submitting);
        let client = AtCoderClient::from_stored_session()?;
        let submission_url = submit_prepared_solution(&workspace, &client, source).await?;
        event(AppEvent::SubmitSuccessful);
        Ok(SubmitOutcome { submission_url })
    }

    pub(crate) async fn login<F>(&self, overwrite: bool, mut event: F) -> Result<LoginOutcome>
    where
        F: FnMut(AppEvent),
    {
        if !overwrite && let Some(result) = verify_current_session().await {
            return Ok(match result {
                Ok(username) => LoginOutcome::AlreadyLoggedIn { username },
                Err(_) => LoginOutcome::ExistingSessionInvalid,
            });
        }

        let revel_session = prompt_revel_session()?;
        event(AppEvent::LoggingIn);
        let client = AtCoderClient::new(Some(&revel_session))?;
        let username = client.validate_session().await?;
        Cookie { revel_session }
            .store()
            .context("Failed to store REVEL_SESSION.")?;
        Ok(LoginOutcome::LoggedIn { username })
    }

    pub(crate) fn logout<F>(&self, mut event: F) -> Result<SessionStatus>
    where
        F: FnMut(AppEvent),
    {
        event(AppEvent::LoggingOut);
        Cookie::set_default().context("Failed to clear REVEL_SESSION.")?;
        Ok(SessionStatus::LoggedOut)
    }

    pub(crate) async fn whoami(&self) -> Result<SessionStatus> {
        Ok(match verify_current_session().await {
            Some(Ok(username)) => SessionStatus::LoggedIn { username },
            Some(Err(_)) => SessionStatus::Invalid,
            None => SessionStatus::LoggedOut,
        })
    }

    pub(crate) fn create_template(&self, request: NewTemplate<'_>) -> Result<TemplateCreated> {
        Ok(TemplateCreated {
            path: create_template(request)?,
        })
    }
}
