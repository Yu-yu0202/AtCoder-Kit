pub const BASE: &str = "https://atcoder.jp";

pub fn settings() -> String {
    format!("{BASE}/settings/")
}

pub fn contest(contest_id: &str) -> String {
    format!("{BASE}/contests/{contest_id}")
}

pub fn tasks(contest_id: &str) -> String {
    format!("{BASE}/contests/{contest_id}/tasks")
}

pub fn problem(contest_id: &str, problem_id: &str) -> String {
    format!("{BASE}/contests/{contest_id}/tasks/{problem_id}")
}

pub fn submit(contest_id: &str) -> String {
    format!("{BASE}/contests/{contest_id}/submit")
}

pub fn submissions(contest_id: &str) -> String {
    format!("{BASE}/contests/{contest_id}/submissions/me?lang=en")
}
