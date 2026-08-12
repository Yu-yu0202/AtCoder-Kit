use crate::client::model::SampleCase;
use anyhow::{Context, Result, anyhow, bail};
use regex::Regex;
use scraper::{ElementRef, Html, Selector};

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct ParsedProblemPage {
    pub(crate) label: String,
    pub(crate) title: String,
    pub(crate) memory_limit_bytes: usize,
    pub(crate) time_limit_msecs: usize,
    pub(crate) sample_cases: Vec<SampleCase>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct TaskRef {
    pub(crate) id: String,
    pub(crate) href: String,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) enum AtCoderPageError {
    ContestNotFound,
    TaskNotFound,
    PermissionDenied,
    SourceTooLong,
    SourceEmpty,
    TurnstileRequired,
    UnknownAlert(String),
}

#[derive(Copy, Clone, Debug, Eq, PartialEq)]
enum SamplePartKind {
    Input,
    Output,
}

fn selector(value: &str) -> Result<Selector> {
    Selector::parse(value).map_err(|_| anyhow!("Failed to compile selector: {value}"))
}

fn element_text(element: ElementRef<'_>) -> String {
    element.text().collect::<String>()
}

pub(crate) fn to_bytes(value: &str, unit: &str) -> Result<usize> {
    let value = value
        .parse::<f64>()
        .context("Failed to parse memory limit.")?;

    match unit.to_ascii_lowercase().as_str() {
        "kib" => Ok((value * 1024.0) as usize),
        "mib" => Ok((value * 1024.0 * 1024.0) as usize),
        "kb" => Ok((value * 1000.0) as usize),
        "mb" => Ok((value * 1000.0 * 1000.0) as usize),
        _ => bail!("Unknown memory unit: {unit}"),
    }
}

pub(crate) fn to_msecs(value: &str, unit: &str) -> Result<usize> {
    let value = value
        .parse::<f64>()
        .context("Failed to parse time limit.")?;

    match unit.to_ascii_lowercase().as_str() {
        "msec" => Ok(value as usize),
        "sec" => Ok((value * 1000.0) as usize),
        _ => bail!("Unknown time unit: {unit}"),
    }
}

fn parse_limits(text: &str) -> Result<(usize, usize)> {
    let limits_re = Regex::new(
        r"(?x)
        (?:実行時間制限|Time\sLimit):\s*
        (?P<t_val>\d+(?:\.\d+)?)\s*(?P<t_unit>[a-zA-Z]+)
        \s*/\s*
        (?:メモリ制限|Memory\sLimit):\s*
        (?P<m_val>\d+(?:\.\d+)?)\s*(?P<m_unit>[a-zA-Z]+)
        ",
    )
    .context("Failed to compile limits regex.")?;
    let limits = limits_re
        .captures(text)
        .context("Failed to parse limits.")?;

    Ok((
        to_msecs(&limits["t_val"], &limits["t_unit"])?,
        to_bytes(&limits["m_val"], &limits["m_unit"])?,
    ))
}

fn sample_kind(heading: &str) -> Option<SamplePartKind> {
    if heading.starts_with("入力例") || heading.starts_with("Sample Input") {
        Some(SamplePartKind::Input)
    } else if heading.starts_with("出力例") || heading.starts_with("Sample Output") {
        Some(SamplePartKind::Output)
    } else {
        None
    }
}

fn parse_sample_cases(document: &Html) -> Result<Vec<SampleCase>> {
    let japanese = selector("span.lang-ja > div.part > section")?;
    let english = selector("span.lang-en > div.part > section")?;
    let heading_selector = selector("h3")?;
    let pre_selector = selector("pre")?;

    let mut sections = document.select(&japanese).collect::<Vec<_>>();
    if sections.is_empty() {
        sections = document.select(&english).collect();
    }

    let mut parts = Vec::new();
    for section in sections {
        let Some(heading) = section.select(&heading_selector).next() else {
            continue;
        };
        let heading = element_text(heading);
        let Some(kind) = sample_kind(heading.trim()) else {
            continue;
        };
        let text = section
            .select(&pre_selector)
            .next()
            .map(element_text)
            .context("Failed to get sample text.")?;
        parts.push((kind, text));
    }

    if parts.is_empty() {
        bail!("Failed to find sample input/output sections.");
    }

    let (pairs, remainder) = parts.as_chunks::<2>();
    let mut samples = Vec::with_capacity(parts.len() / 2);
    for pair in pairs {
        let [
            (SamplePartKind::Input, input),
            (SamplePartKind::Output, expected),
        ] = pair
        else {
            bail!("Sample input/output order is invalid.");
        };
        samples.push(SampleCase {
            input: input.clone(),
            expected: expected.clone(),
        });
    }
    if !remainder.is_empty() {
        bail!("Sample input/output pair is incomplete.");
    }

    Ok(samples)
}

pub(crate) fn parse_problem_page(html: &str) -> Result<ParsedProblemPage> {
    let document = Html::parse_document(html);
    let title_selector = selector("span.h2")?;
    let title_text = document
        .select(&title_selector)
        .next()
        .map(element_text)
        .context("Failed to get problem title.")?;
    let (label, title) = title_text
        .trim()
        .split_once(" - ")
        .context("Failed to parse problem title.")?;

    let limits_selector = selector("div:has(> #task-statement) > p")?;
    let limits_text = document
        .select(&limits_selector)
        .next()
        .map(element_text)
        .context("Failed to get limits.")?;
    let (time_limit_msecs, memory_limit_bytes) = parse_limits(&limits_text)?;

    Ok(ParsedProblemPage {
        label: label.trim().to_string(),
        title: title.trim().to_string(),
        memory_limit_bytes,
        time_limit_msecs,
        sample_cases: parse_sample_cases(&document)?,
    })
}

pub(crate) fn parse_task_refs(html: &str, contest_id: &str) -> Result<Vec<TaskRef>> {
    let document = Html::parse_document(html);
    let row_selector = selector("#main-container tbody tr")?;
    let link_selector = selector("a[href]")?;
    let expected_prefix = format!("/contests/{contest_id}/tasks/");
    let mut tasks = Vec::new();

    for row in document.select(&row_selector) {
        let link = row
            .select(&link_selector)
            .find(|link| {
                link.value()
                    .attr("href")
                    .is_some_and(|href| href.starts_with(&expected_prefix))
            })
            .context("Failed to get task information.")?;
        let href = link
            .value()
            .attr("href")
            .context("Failed to get task URL.")?;
        let id = href
            .strip_prefix(&expected_prefix)
            .filter(|id| !id.is_empty() && !id.contains('/'))
            .context("Failed to parse task ID.")?;
        if tasks.iter().any(|task: &TaskRef| task.id == id) {
            bail!("Duplicate task ID: {id}");
        }
        tasks.push(TaskRef {
            id: id.to_string(),
            href: href.to_string(),
        });
    }

    if tasks.is_empty() {
        bail!("Failed to find any contest tasks.");
    }

    Ok(tasks)
}

pub(crate) fn parse_contest_title(html: &str) -> Result<String> {
    let document = Html::parse_document(html);
    document
        .select(&selector("h1")?)
        .next()
        .map(element_text)
        .map(|title| title.trim().to_string())
        .context("Failed to get contest title.")
}

pub(crate) fn parse_alert(html: &str) -> Result<Option<String>> {
    let document = Html::parse_document(html);
    Ok(document
        .select(&selector("div#main-container > div > div.alert")?)
        .next()
        .map(element_text)
        .map(|text| text.trim().to_string()))
}

pub(crate) fn classify_alert(alert: &str) -> AtCoderPageError {
    if alert.contains("Contest not found") || alert.contains("指定されたコンテストが見つかりません")
    {
        AtCoderPageError::ContestNotFound
    } else if alert.contains("Task not found") || alert.contains("指定されたタスクが見つかりません")
    {
        AtCoderPageError::TaskNotFound
    } else if alert.contains("Permission denied") || alert.contains("権限がありません") {
        AtCoderPageError::PermissionDenied
    } else if alert.contains("The source code is too long")
        || alert.contains("ソースコードが長すぎます")
    {
        AtCoderPageError::SourceTooLong
    } else if alert.contains("The source code must not be empty")
        || alert.contains("ソースコードが空です")
    {
        AtCoderPageError::SourceEmpty
    } else if alert.contains("Error") || alert.contains("エラーが発生しました") {
        AtCoderPageError::TurnstileRequired
    } else {
        AtCoderPageError::UnknownAlert(alert.to_string())
    }
}

pub(crate) fn parse_csrf_token(html: &str) -> Result<String> {
    let document = Html::parse_document(html);
    document
        .select(&selector(r#"input[name="csrf_token"]"#)?)
        .next()
        .and_then(|input| input.value().attr("value"))
        .map(str::to_string)
        .context("Failed to get CSRF token.")
}

pub(crate) fn parse_submission_detail_href(html: &str) -> Result<String> {
    let document = Html::parse_document(html);
    document
        .select(&selector("a.submission-details-link")?)
        .next()
        .and_then(|link| link.value().attr("href"))
        .map(str::to_string)
        .context("Failed to get submission details URL.")
}

pub(crate) fn parse_username(html: &str) -> Result<String> {
    let document = Html::parse_document(html);
    document
        .select(&selector(r#"[id="ui.UserName"]"#)?)
        .next()
        .and_then(|input| input.value().attr("value"))
        .map(str::to_string)
        .context("Failed to get username. Maybe AtCoder's website structure has changed?")
}

#[cfg(test)]
mod tests {
    use super::*;

    const PROBLEM_JA: &str = include_str!("../../tests/fixtures/atcoder/problem_ja.html");
    const PROBLEM_EN: &str = include_str!("../../tests/fixtures/atcoder/problem_en.html");
    const TASKS: &str = include_str!("../../tests/fixtures/atcoder/tasks.html");
    const SUBMIT_FORM: &str = include_str!("../../tests/fixtures/atcoder/submit_form.html");
    const SUBMISSIONS: &str = include_str!("../../tests/fixtures/atcoder/submissions.html");
    const SETTINGS: &str = include_str!("../../tests/fixtures/atcoder/settings.html");

    #[test]
    fn parses_japanese_problem_page() {
        let page = parse_problem_page(PROBLEM_JA).unwrap();
        assert_eq!(page.label, "A");
        assert_eq!(page.title, "Synthetic Problem - With Dash");
        assert_eq!(page.time_limit_msecs, 2_000);
        assert_eq!(page.memory_limit_bytes, 1_073_741_824);
        assert_eq!(page.sample_cases.len(), 2);
        assert_eq!(page.sample_cases[0].input, "1 2\n");
        assert_eq!(page.sample_cases[0].expected, "3\n");
    }

    #[test]
    fn falls_back_to_english_problem_statement() {
        let page = parse_problem_page(PROBLEM_EN).unwrap();
        assert_eq!(page.time_limit_msecs, 500);
        assert_eq!(page.memory_limit_bytes, 64_000);
        assert_eq!(page.sample_cases.len(), 1);
    }

    #[test]
    fn rejects_unpaired_samples_without_panicking() {
        let html = PROBLEM_JA.replace("<section><h3>出力例 2</h3><pre>30\n</pre></section>", "");
        assert!(parse_problem_page(&html).is_err());
    }

    #[test]
    fn rejects_pages_where_sample_or_task_selectors_no_longer_match() {
        let without_samples = PROBLEM_JA.replace("lang-ja", "changed-language-class");
        assert!(parse_problem_page(&without_samples).is_err());
        assert!(parse_task_refs("<html><body>not a tasks page</body></html>", "abc999").is_err());
    }

    #[test]
    fn converts_supported_limit_units() {
        assert_eq!(to_msecs("1.5", "sec").unwrap(), 1_500);
        assert_eq!(to_msecs("250", "msec").unwrap(), 250);
        assert_eq!(to_bytes("1", "KiB").unwrap(), 1_024);
        assert_eq!(to_bytes("2", "MB").unwrap(), 2_000_000);
        assert!(to_bytes("1", "GiB").is_err());
    }

    #[test]
    fn parses_task_references_and_rejects_malformed_rows() {
        let tasks = parse_task_refs(TASKS, "abc999").unwrap();
        assert_eq!(
            tasks
                .iter()
                .map(|task| task.id.as_str())
                .collect::<Vec<_>>(),
            ["abc999_a", "abc999_b"]
        );

        let malformed = TASKS.replace(
            "<td><a href=\"/contests/abc999/tasks/abc999_b\">B</a></td>",
            "<td>missing link</td>",
        );
        assert!(parse_task_refs(&malformed, "abc999").is_err());
    }

    #[test]
    fn classifies_bilingual_alerts() {
        assert_eq!(
            classify_alert("Contest not found"),
            AtCoderPageError::ContestNotFound
        );
        assert_eq!(
            classify_alert("権限がありません"),
            AtCoderPageError::PermissionDenied
        );
    }

    #[test]
    fn parses_submission_and_settings_fields() {
        assert_eq!(parse_csrf_token(SUBMIT_FORM).unwrap(), "synthetic-token");
        assert_eq!(
            parse_submission_detail_href(SUBMISSIONS).unwrap(),
            "/contests/abc999/submissions/123"
        );
        assert_eq!(parse_username(SETTINGS).unwrap(), "fixture-user");
    }
}
