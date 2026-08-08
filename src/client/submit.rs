use crate::client::auth::get_token_header;
use crate::client::endpoints;
use crate::core::network::CLIENT;
use anyhow::*;
use http::StatusCode;
use scraper::{Html, Selector};
use serde::Serialize;
use std::env;
use std::process::Stdio;
use tokio::{fs, io, io::AsyncBufReadExt, process};

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

async fn read_str_crlf(file_path: &str) -> Result<String> {
    let file = fs::File::open(file_path)
        .await
        .context("Failed to open source file.")?;
    let reader = io::BufReader::new(file);
    let metadata = fs::metadata(file_path)
        .await
        .context("Failed to get file metadata.")?;

    let mut source_code = String::with_capacity(metadata.len() as usize);

    let mut lines = reader.lines();
    while let Some(line) = lines.next_line().await.context("Failed to read line.")? {
        source_code.push_str(&line);
        source_code.push_str("\r\n");
    }
    Ok(source_code)
}

pub async fn submit() -> Result<String> {
    let template_config = crate::core::template::from_file()?;
    let contest = crate::client::contest::from_file()?;
    let problem_name = env::current_dir()
        .context("Failed to get current directory.")?
        .file_name()
        .context("Failed to get current directory name.")?
        .to_string_lossy()
        .to_uppercase();
    let problem = contest.problems.get(&problem_name).context(format!(
        "Failed to get problem {} from contest.",
        problem_name
    ))?;

    if let Some(pre_submit) = template_config.pre_submit.as_deref().filter(|s| !s.is_empty()) {
        let mut cmd = process::Command::new(&pre_submit[0]);
        if pre_submit.len() > 1 {
            cmd.args(&pre_submit[1..]);
        }
        cmd.stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());

        let result = cmd
            .spawn()
            .context("Failed to run pre-submit command.")?
            .wait_with_output()
            .await
            .context("Failed to run pre-submit command.")?;

        if !result.status.success() {
            let stdout = String::from_utf8_lossy(&result.stdout);
            let stderr = String::from_utf8_lossy(&result.stderr);

            bail!(
                "Failed to run pre-submit command.\nexit code: {}\nstdout:\n{}\nstderr:\n{}",
                result.status.code().unwrap_or(-1),
                stdout,
                stderr
            );
        }
    }

    let problem_res = CLIENT
        .get(&problem.url)
        .headers(get_token_header()?)
        .send()
        .await
        .context(format!(
            "Failed to get problem {} from contest.",
            problem_name
        ))?;

    let status = problem_res.status();

    let problem_document = Html::parse_document(
        &problem_res
            .text()
            .await
            .context("Failed to parse document.")?,
    );

    if status == StatusCode::NOT_FOUND {
        let error_selector = Selector::parse("div#main-container > div > div.alert")
            .map_err(|_| anyhow!("Failed to fetch task: 404(Failed to get error message)"))?;
        let error_text = problem_document
            .select(&error_selector)
            .next()
            .context("Failed to fetch task: 404(Failed to get error message)")?
            .text()
            .collect::<Vec<_>>()
            .join("");
        let error_text = error_text.trim();

        if error_text.contains("Contest not found")
            || error_text.contains("指定されたコンテストが見つかりません")
        {
            bail!("Failed to fetch task: Contest not found.");
        }

        if error_text.contains("Task not found")
            || error_text.contains("指定されたタスクが見つかりません")
        {
            bail!("Failed to fetch task: Task not found.");
        }

        if error_text.contains("Permission denied") || error_text.contains("権限がありません")
        {
            bail!(r#"Failed to fetch task: You are not logged in. Run "ackit login" first."#);
        }

        bail!("Failed to fetch task: 404(Failed to get error message)");
    }

    let csrf_token_selector = Selector::parse(r#"input[name="csrf_token"]"#)
        .map_err(|_| anyhow!("Failed to compile selector."))?;

    let csrf_token = problem_document
        .select(&csrf_token_selector)
        .next()
        .context("Failed to get CSRF token.")?
        .value()
        .attr("value")
        .context("Failed to get CSRF token.")?;

    let source_file = template_config
        .submit_file
        .to_str()
        .context("Failed to read source code: submit_file includes non UTF-8 charters.")?;
    let source_code = read_str_crlf(&source_file).await?;

    let payload = SubmitData {
        task_name: problem.id.to_string(),
        language_id: template_config.language_id.to_string(),
        source_code,
        csrf_token: csrf_token.to_string(),
    };

    let submit_res = CLIENT
        .post(endpoints::submit(&contest.id))
        .headers(get_token_header()?)
        .form(&payload)
        .send()
        .await
        .context("Failed to submit task.")?;

    let status = submit_res.status();

    let submit_document = Html::parse_document(
        &submit_res
            .text()
            .await
            .context("Failed to parse document.")?,
    );
    let error_selector = Selector::parse("div#main-container > div > div.alert")
        .map_err(|_| anyhow!("Failed to parse selector."))?;

    if let Some(error_text) = submit_document.select(&error_selector).next() {
        let error_text = error_text.text().collect::<Vec<_>>().join("");
        let error_text = error_text.trim();

        if error_text.contains("Contest not found")
            || error_text.contains("指定されたコンテストが見つかりません")
        {
            bail!("Failed to submit task: Contest not found.");
        }

        if error_text.contains("Task not found")
            || error_text.contains("指定されたタスクが見つかりません")
        {
            bail!("Failed to submit task: Task not found.");
        }

        if error_text.contains("Permission denied") || error_text.contains("権限がありません")
        {
            bail!(r#"Failed to submit task: You are not logged in. Run "ackit login" first."#);
        }

        if error_text.contains("Error") || error_text.contains("エラーが発生しました") {
            bail!("Failed to submit task: Error. (Maybe Cloudflare Turnstile Required)");
        }

        if error_text.contains("The source code is too long")
            || error_text.contains("ソースコードが長すぎます")
        {
            bail!("Failed to submit task: Source code is too long.");
        }

        if error_text.contains("The source code must not be empty")
            || error_text.contains("ソースコードが空です")
        {
            bail!("Failed to submit task: Source code is empty.");
        }

        bail!("Failed to submit task.");
    }

    if !status.is_success() {
        bail!("Failed to submit task.");
    }

    let submissions_res = CLIENT
        .get(endpoints::submissions(&contest.id))
        .headers(get_token_header()?)
        .send()
        .await
        .context("Failed to get submissions.")?;

    if !(&submissions_res.status()).is_success() {
        bail!("Failed to get submissions.");
    }

    let submissions_document = Html::parse_document(
        &submissions_res
            .text()
            .await
            .context("Failed to parse document.")?,
    );

    let details_selector = Selector::parse("a.submission-details-link")
        .map_err(|_| anyhow!("Failed to parse selector."))?;

    let details = submissions_document
        .select(&details_selector)
        .nth(0)
        .context("Failed to get submissions.")?;

    let details_url = details.attr("href").context("Failed to get submissions.")?;

    Ok(details_url.to_string())
}
