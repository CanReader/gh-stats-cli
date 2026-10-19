use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use serde_json::json;

use crate::github::Client;

const CONTRIBUTIONS: &str = r#"
query($login: String!) {
  user(login: $login) {
    contributionsCollection {
      totalCommitContributions
      totalPullRequestContributions
      totalIssueContributions
      totalPullRequestReviewContributions
      restrictedContributionsCount
      contributionCalendar {
        totalContributions
        weeks { contributionDays { date contributionCount contributionLevel } }
      }
    }
  }
}"#;

const LANGUAGES: &str = r#"
query($login: String!, $after: String, $privacy: RepositoryPrivacy, $isFork: Boolean) {
  repositoryOwner(login: $login) {
    repositories(first: 100, after: $after, ownerAffiliations: OWNER, privacy: $privacy, isFork: $isFork) {
      pageInfo { hasNextPage endCursor }
      nodes {
        createdAt
        languages(first: 20, orderBy: {field: SIZE, direction: DESC}) {
          edges { size node { name color } }
        }
      }
    }
  }
}"#;

#[derive(Debug, Clone, Serialize)]
pub struct Day {
    pub date: String,
    pub count: u32,
    pub level: u8,
}

#[derive(Debug, Clone, Serialize)]
pub struct Contributions {
    pub total: u32,
    pub commits: u32,
    pub pull_requests: u32,
    pub issues: u32,
    pub reviews: u32,
    pub private: u32,
    pub days: Vec<Day>,
}

#[derive(Debug, Clone, Serialize)]
pub struct LangSize {
    pub name: String,
    pub bytes: u64,
    pub color: Option<String>,
}

#[derive(Debug, Clone)]
pub struct RepoLangs {
    pub created_at: String,
    pub langs: Vec<LangSize>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct RawCollection {
    total_commit_contributions: u32,
    total_pull_request_contributions: u32,
    total_issue_contributions: u32,
    total_pull_request_review_contributions: u32,
    restricted_contributions_count: u32,
    contribution_calendar: RawCalendar,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct RawCalendar {
    total_contributions: u32,
    weeks: Vec<RawWeek>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct RawWeek {
    contribution_days: Vec<RawDay>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct RawDay {
    date: String,
    contribution_count: u32,
    contribution_level: String,
}

pub fn contributions(client: &Client, login: &str) -> Result<Contributions> {
    let data = client.graphql(CONTRIBUTIONS, json!({ "login": login }))?;
    let raw: RawCollection =
        serde_json::from_value(data["user"]["contributionsCollection"].clone())
            .context("unexpected shape for contributions")?;

    let days = raw
        .contribution_calendar
        .weeks
        .into_iter()
        .flat_map(|w| w.contribution_days)
        .map(|d| Day {
            level: level(&d.contribution_level),
            date: d.date,
            count: d.contribution_count,
        })
        .collect();

    Ok(Contributions {
        total: raw.contribution_calendar.total_contributions,
        commits: raw.total_commit_contributions,
        pull_requests: raw.total_pull_request_contributions,
        issues: raw.total_issue_contributions,
        reviews: raw.total_pull_request_review_contributions,
        private: raw.restricted_contributions_count,
        days,
    })
}

pub fn languages(
    client: &Client,
    login: &str,
    with_private: bool,
    with_forks: bool,
) -> Result<Vec<RepoLangs>> {
    let mut repos = Vec::new();
    let mut after: Option<String> = None;
    loop {
        let vars = json!({
            "login": login,
            "after": after,
            "privacy": if with_private { None } else { Some("PUBLIC") },
            "isFork": if with_forks { None } else { Some(false) },
        });
        let data = client.graphql(LANGUAGES, vars)?;
        let conn = &data["repositoryOwner"]["repositories"];

        for node in conn["nodes"].as_array().into_iter().flatten() {
            let langs = node["languages"]["edges"]
                .as_array()
                .into_iter()
                .flatten()
                .filter_map(|e| {
                    Some(LangSize {
                        name: e["node"]["name"].as_str()?.to_string(),
                        bytes: e["size"].as_u64()?,
                        color: e["node"]["color"].as_str().map(str::to_string),
                    })
                })
                .collect();
            repos.push(RepoLangs {
                created_at: node["createdAt"].as_str().unwrap_or_default().to_string(),
                langs,
            });
        }

        let page = &conn["pageInfo"];
        if page["hasNextPage"].as_bool() != Some(true) {
            break;
        }
        after = page["endCursor"].as_str().map(str::to_string);
        if after.is_none() {
            break;
        }
    }
    Ok(repos)
}

fn level(name: &str) -> u8 {
    match name {
        "FIRST_QUARTILE" => 1,
        "SECOND_QUARTILE" => 2,
        "THIRD_QUARTILE" => 3,
        "FOURTH_QUARTILE" => 4,
        _ => 0,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn levels() {
        assert_eq!(level("NONE"), 0);
        assert_eq!(level("THIRD_QUARTILE"), 3);
        assert_eq!(level("FOURTH_QUARTILE"), 4);
        assert_eq!(level("something new"), 0);
    }

    #[test]
    fn parses_calendar() {
        let raw: RawCollection = serde_json::from_value(json!({
            "totalCommitContributions": 10,
            "totalPullRequestContributions": 2,
            "totalIssueContributions": 1,
            "totalPullRequestReviewContributions": 3,
            "restrictedContributionsCount": 4,
            "contributionCalendar": {
                "totalContributions": 20,
                "weeks": [{ "contributionDays": [
                    { "date": "2026-10-04", "contributionCount": 0, "contributionLevel": "NONE" },
                    { "date": "2026-10-05", "contributionCount": 7, "contributionLevel": "FOURTH_QUARTILE" }
                ]}]
            }
        }))
        .unwrap();
        assert_eq!(
            raw.contribution_calendar.weeks[0].contribution_days.len(),
            2
        );
        assert_eq!(raw.total_pull_request_review_contributions, 3);
    }
}
