use crate::client::auth::get_token_header;
use crate::client::endpoints;
use crate::core::network::CLIENT;
use crate::core::template::Template;
use anyhow::*;
use regex::Regex;
use reqwest::StatusCode;
use scraper::{Html, Selector};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::PathBuf;
use std::{env, fs, io, path::Path};

fn to_bytes(value: &str, unit: &str) -> Result<usize> {
    let value = value
        .parse::<f64>()
        .context("Failed to parse memory limit.")?;

    match unit.to_lowercase().as_str() {
        "kib" => Ok((value * 1024.0) as usize),
        "mib" => Ok((value * 1024.0 * 1024.0) as usize),
        "kb" => Ok((value * 1000.0) as usize),
        "mb" => Ok((value * 1000.0 * 1000.0) as usize),
        _ => bail!("Unknown memory unit: {}", unit),
    }
}

fn to_msecs(value: &str, unit: &str) -> Result<usize> {
    let value = value
        .parse::<f64>()
        .context("Failed to parse time limit.")?;

    match unit.to_lowercase().as_str() {
        "msec" => Ok(value as usize),
        "sec" => Ok((value * 1000.0) as usize),
        _ => bail!("Unknown time unit: {}", unit),
    }
}

fn copy_recursive(src: &Path, dst: &Path) -> Result<()> {
    if src.is_file() {
        if let Some(parent) = dst.parent() {
            fs::create_dir_all(parent)?;
        }
        fs::copy(src, dst)?;
    } else {
        fs::create_dir_all(dst)?;
        for entry in fs::read_dir(src)? {
            let entry = entry?;
            let path = entry.path();
            let dst_path = dst.join(entry.file_name());
            copy_recursive(&path, &dst_path)?;
        }
    }
    Ok(())
}

#[derive(Default, Serialize, Deserialize, Debug)]
pub enum ContestType {
    ABC,
    ARC,
    AGC,
    AHC,
    AWC,
    #[default]
    Other,
}

#[derive(Default, Serialize, Deserialize, Debug)]
pub struct SampleCase {
    pub input: String,
    pub expected: String,
}

#[derive(Default, Serialize, Deserialize, Debug)]
pub struct Problem {
    pub id: String,
    pub contest_id: String,
    pub label: String,
    pub url: String,
    pub title: String,

    pub memory_limit_bytes: usize,
    pub time_limit_msecs: usize,

    pub sample_cases: Vec<SampleCase>,
}

#[derive(Default, Serialize, Deserialize, Debug)]
pub struct Contest {
    pub id: String,
    pub contest_type: ContestType,
    pub title: String,

    pub problems: HashMap<String, Problem>,
}

impl SampleCase {
    pub fn extract(problem_document: &Html) -> Result<Vec<SampleCase>> {
        let mut sample_cases: Vec<SampleCase> = Vec::new();

        let sample_cases_selector = Selector::parse("span.lang-ja > div.part > section")
            .map_err(|_| anyhow!("Failed to compile selector."))?;

        let matched_sample_cases = problem_document
            .select(&sample_cases_selector)
            .filter(|section| {
                section.select(&Selector::parse("h3").unwrap()).any(|h3| {
                    let text = h3.text().collect::<String>();
                    text.starts_with("入力例")
                        || text.starts_with("Sample Input")
                        || text.starts_with("出力例")
                        || text.starts_with("Sample Output")
                })
            })
            .collect::<Vec<_>>();

        for sample_case in matched_sample_cases.chunks(2) {
            let text_selector =
                Selector::parse("pre").map_err(|_| anyhow!("Failed to compile selector."))?;

            let input = sample_case[0]
                .select(&text_selector)
                .next()
                .context("Failed to get sample input.")?
                .text()
                .collect::<String>();

            let expected = sample_case[1]
                .select(&text_selector)
                .next()
                .context("Failed to get sample output.")?
                .text()
                .collect::<String>();

            sample_cases.push(SampleCase { input, expected });
        }

        Ok(sample_cases)
    }
}

impl Problem {
    pub async fn fetch(contest_id: &str, problem_id: &str) -> Result<Problem> {
        let mut problem = Problem {
            id: problem_id.to_string(),
            contest_id: contest_id.to_string(),
            url: endpoints::problem(contest_id, problem_id),
            ..Problem::default()
        };

        let problem_res = CLIENT
            .get(&problem.url)
            .headers(get_token_header()?)
            .send()
            .await
            .context("Failed to contact server.")?;

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

        let title_selector =
            Selector::parse("span.h2").map_err(|_| anyhow!("Failed to compile selector."))?;

        let title = (&problem_document)
            .select(&title_selector)
            .next()
            .context("Failed to parse title.")?
            .text()
            .next()
            .context("Failed to get title.")?
            .rsplit_once(" - ")
            .context("Failed to parse title.")?; // (label, title)

        problem.label = title.0.trim().to_string();
        problem.title = title.1.trim().to_string();

        let limits_selector = Selector::parse("div:has(> #task-statement) > p")
            .map_err(|_| anyhow!("Failed to compile selector."))?;

        let limits_text = (&problem_document)
            .select(&limits_selector)
            .next()
            .context("Failed to parse limits.")?
            .text()
            .next()
            .context("Failed to get limits.")?;

        let limits_re = Regex::new(
            r"(?x)
(?:実行時間制限|Time\sLimit):\s*
(?P<t_val>\d+(?:\.\d+)?)\s*(?P<t_unit>[a-zA-Z]+)
\s*/\s*
(?:メモリ制限|Memory\sLimit):\s*
(?P<m_val>\d+(?:\.\d+)?)\s*(?P<m_unit>[a-zA-Z]+)
",
        )
        .context("Failed to compile regex.")?;

        let limits = limits_re.captures(limits_text);

        let limits = limits.context("Failed to parse limits.")?;

        problem.time_limit_msecs = to_msecs(&limits["t_val"], &limits["t_unit"])?;
        problem.memory_limit_bytes = to_bytes(&limits["m_val"], &limits["m_unit"])?;

        problem.sample_cases = SampleCase::extract(&problem_document)?;

        Ok(problem)
    }

    pub async fn fetch_all(contest_id: &str) -> Result<HashMap<String, Problem>> {
        let mut problems: HashMap<String, Problem> = HashMap::new();

        let tasks_res = CLIENT
            .get(endpoints::tasks(contest_id))
            .headers(get_token_header()?)
            .send()
            .await
            .context("Failed to contact server.")?;

        let status = tasks_res.status();

        let tasks_document = Html::parse_document(
            &tasks_res
                .text()
                .await
                .context("Failed to parse document.")?,
        );

        if status == StatusCode::NOT_FOUND {
            let error_selector = Selector::parse("div#main-container > div > div.alert")
                .map_err(|_| anyhow!("Failed to fetch tasks: 404(Failed to get error message)"))?;
            let error_text = tasks_document
                .select(&error_selector)
                .next()
                .context("Failed to fetch tasks: 404(Failed to get error message)")?
                .text()
                .collect::<Vec<_>>()
                .join("");
            let error_text = error_text.trim();

            if error_text.contains("Contest not found")
                || error_text.contains("指定されたコンテストが見つかりません")
            {
                bail!("Failed to fetch tasks: Contest not found.");
            }

            if error_text.contains("Permission denied") || error_text.contains("権限がありません")
            {
                bail!(r#"Failed to fetch tasks: You are not logged in. Run "ackit login" first."#);
            }

            bail!("Failed to fetch tasks: 404(Failed to get error message)");
        }

        let tasks_selector = Selector::parse("#main-container tbody tr")
            .map_err(|_| anyhow!("Failed to compile selector."))?;

        let task_description_selector =
            Selector::parse("td").map_err(|_| anyhow!("Failed to compile selector."))?;

        let tasks = tasks_document.select(&tasks_selector);

        for problem_element in tasks {
            let descriptions: Vec<_> = problem_element.select(&task_description_selector).collect();
            let task = descriptions[0]
                .select(&Selector::parse("a").map_err(|_| anyhow!("Failed to compile selector."))?)
                .next()
                .context("Failed to get task information.")?;
            let task_url = (&task).attr("href").context("Failed to get task url.")?;
            let task_id = task_url
                .rsplit_once('/')
                .context("Failed to get task id.")?
                .1;

            let problem = Problem::fetch(contest_id, task_id).await?;
            problems.insert(problem.label.clone(), problem);
        }

        Ok(problems)
    }
}

impl Contest {
    pub async fn fetch(contest_id: &str) -> Result<Contest> {
        let mut contest = Contest::default();

        let contest_res = CLIENT
            .get(endpoints::contest(contest_id))
            .headers(get_token_header()?)
            .send()
            .await
            .context("Failed to contact server.")?;

        if contest_res.status() == StatusCode::NOT_FOUND {
            bail!("Contest not found.");
        }

        let contest_document = Html::parse_document(
            &contest_res
                .text()
                .await
                .context("Failed to parse document.")?,
        );

        let contest_title_selector =
            Selector::parse("h1").map_err(|_| anyhow!("Failed to compile selector."))?;

        let contest_title = contest_document
            .select(&contest_title_selector)
            .next()
            .ok_or_else(|| anyhow!("Failed to get title."))?
            .text()
            .next()
            .ok_or_else(|| anyhow!("Failed to get title."))?;

        let contest_type = match &contest_id[..3.min(contest_id.len())] {
            "abc" => ContestType::ABC,
            "arc" => ContestType::ARC,
            "agc" => ContestType::AGC,
            "ahc" => ContestType::AHC,
            "awc" => ContestType::AWC,
            _ => ContestType::Other,
        };

        contest.id = contest_id.to_string();
        contest.contest_type = contest_type;
        contest.title = contest_title.to_string();
        contest.problems = Problem::fetch_all(contest_id).await?;

        Ok(contest)
    }
}

fn find_project_root() -> Result<PathBuf> {
    let mut current_dir = env::current_dir().context("Failed to get current directory.")?;

    loop {
        if current_dir.join("contest.json").exists() {
            return Ok(current_dir);
        }

        if !current_dir.pop() {
            bail!("Failed to find `contest.json` directory.");
        }
    }
}

pub fn save_contest(
    contest: Contest,
    template_name: Option<&str>,
    no_template: bool,
) -> Result<()> {
    let contest_dir = Path::new(&contest.id);
    let json_path = contest_dir.join("contest.json");

    if !contest_dir.exists() {
        fs::create_dir_all(&contest_dir)
            .context("Failed to save contest: Failed to create directory.")?;
    }

    let file = fs::File::create(&json_path)
        .context("Failed to save contest: Failed to create 'contest.json'.")?;
    let writer = io::BufWriter::new(file);

    serde_json::to_writer_pretty(writer, &contest)
        .context("Failed to save contest: Failed to write 'contest.json'.")?;

    let store = Template::load()?;

    if no_template {
        return Ok(());
    }

    let template = template_name
        .map(|name| store.get(name).context("Template not found."))
        .transpose()?
        .or_else(|| store.get_default());

    let Some(template) = template else {
        return Ok(());
    };

    for (label, _) in contest.problems {
        fs::create_dir_all(contest_dir.join(&label.to_lowercase()))
            .context("Failed to save contest: Failed to create directory.")?;

        copy_recursive(
            &template.template_path,
            &contest_dir.join(&label.to_lowercase()),
        )
        .context("Failed to save contest: Failed to copy template.")?;
    }

    Ok(())
}

pub fn from_file() -> Result<Contest> {
    let root = find_project_root()?;
    let json_path = root.join("contest.json");

    let file = fs::File::open(&json_path)
        .context("Failed to read contest: Failed to open `contest.json`.")?;
    let reader = io::BufReader::new(file);

    let contest = serde_json::from_reader(reader)
        .context("Failed to read contest: Failed to parse `contest.json`.")?;

    Ok(contest)
}
