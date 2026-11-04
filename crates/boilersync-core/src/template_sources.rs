//! Template references (`org/repo#subdir`, GitHub URLs) and their local
//! checkouts under the template cache.

use std::path::{Path, PathBuf};

use crate::error::{Error, Result};
use crate::git;
use crate::paths::Paths;

/// Repo locator assumed for legacy `.boilersync` refs without a source.
pub const DEFAULT_LEGACY_REPO_LOCATOR: &str = "openbase-community/templates";
/// `(org, repo)` identity of a template source repository.
type RepoIdentity = (&'static str, &'static str);
/// Renamed template repositories: old identity -> current identity.
const REPO_RENAMES: [(RepoIdentity, RepoIdentity); 1] =
    [(("openbase-community", "openbase-boilerplate"), ("openbase-community", "templates"))];

/// A resolved template: where its source repository is checked out and which
/// directory inside it holds the template.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TemplateSource {
    /// Canonical ref: `https://github.com/org/repo.git#subdir`.
    pub ref_: String,
    pub template_dir: PathBuf,
    pub repo_url: String,
    pub org: String,
    pub repo: String,
    pub subdir: String,
    pub local_repo_path: PathBuf,
}

impl TemplateSource {
    /// Stable identity used for cycle detection.
    pub fn identifier(&self) -> String {
        format!("source:{}/{}#{}", self.org, self.repo, self.subdir)
    }

    pub fn canonical_ref(&self) -> String {
        format!("{}#{}", self.repo_url, self.subdir)
    }
}

/// A parsed `org/repo` or GitHub URL.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RepoLocator {
    pub org: String,
    pub repo: String,
    pub clone_url: String,
}

fn canonical_repo_identity(org: &str, repo: &str) -> (String, String) {
    for ((from_org, from_repo), (to_org, to_repo)) in REPO_RENAMES {
        if org == from_org && repo == from_repo {
            return (to_org.to_string(), to_repo.to_string());
        }
    }
    (org.to_string(), repo.to_string())
}

fn locator(org: &str, repo: &str) -> RepoLocator {
    let (org, repo) = canonical_repo_identity(org, repo);
    let clone_url = format!("https://github.com/{org}/{repo}.git");
    RepoLocator { org, repo, clone_url }
}

/// Parse `org/repo`, `https://github.com/org/repo` or `...repo.git`.
pub fn parse_repo_locator(repo_locator: &str) -> Result<RepoLocator> {
    let text = repo_locator.trim();
    if text.is_empty() {
        return Err(Error::Invalid("Template source repo cannot be empty.".to_string()));
    }
    if text.starts_with("git@") {
        return Err(Error::Invalid("SSH template refs are not supported. Use a GitHub HTTPS URL.".to_string()));
    }
    if let Some((scheme, rest)) = text.split_once("://") {
        if scheme != "https" {
            return Err(Error::Invalid(format!("Invalid template source repo URL: {repo_locator}")));
        }
        let (host, path) = rest.split_once('/').unwrap_or((rest, ""));
        if host != "github.com" && host != "www.github.com" {
            return Err(Error::Invalid("Template source repo URL must be hosted on github.com.".to_string()));
        }
        let path = path.split(['?', '#']).next().unwrap_or("").trim_matches('/');
        let path = path.strip_suffix(".git").unwrap_or(path);
        let parts: Vec<&str> = path.split('/').filter(|part| !part.is_empty()).collect();
        if parts.len() != 2 {
            return Err(Error::Invalid(
                "Template source repo URL must be in the format https://github.com/org/repo(.git).".to_string(),
            ));
        }
        return Ok(locator(parts[0], parts[1]));
    }
    if let Some((org, repo)) = text.split_once('/')
        && !org.is_empty()
        && !repo.is_empty()
        && !org.contains(char::is_whitespace)
        && !repo.contains(char::is_whitespace)
        && !repo.contains('/')
    {
        let repo = repo.strip_suffix(".git").unwrap_or(repo);
        return Ok(locator(org, repo));
    }
    Err(Error::Invalid("Invalid template source repo. Use org/repo or https://github.com/org/repo(.git).".to_string()))
}

fn normalize_subdir(subdir: &str) -> Result<String> {
    let cleaned = subdir.trim().trim_matches('/');
    if cleaned.is_empty() {
        return Err(Error::Invalid("Template source ref must include a non-empty subdir after '#'.".to_string()));
    }
    if cleaned.starts_with("../") || cleaned == ".." || cleaned.contains("/../") {
        return Err(Error::Invalid("Template source subdir must stay within the source repository.".to_string()));
    }
    Ok(cleaned.to_string())
}

/// Split `locator#subdir`.
pub fn split_source_ref(template_ref: &str) -> Result<(String, String)> {
    let Some((repo_locator, subdir)) = template_ref.split_once('#') else {
        return Err(Error::Invalid(
            "Template source ref must be in the format https://github.com/org/repo.git#subdir.".to_string(),
        ));
    };
    let repo_locator = repo_locator.trim();
    if repo_locator.is_empty() {
        return Err(Error::Invalid("Template source ref must include a repo before '#'.".to_string()));
    }
    Ok((repo_locator.to_string(), normalize_subdir(subdir)?))
}

fn ensure_repo_cloned(repo_url: &str, repo_path: &Path) -> Result<()> {
    if let Some(parent) = repo_path.parent() {
        std::fs::create_dir_all(parent).map_err(|e| Error::io(parent, e))?;
    }
    if repo_path.exists() {
        if repo_path.join(".git").exists() {
            return Ok(());
        }
        if std::fs::read_dir(repo_path).map_err(|e| Error::io(repo_path, e))?.next().is_some() {
            return Err(Error::AlreadyExists(format!(
                "Template repo path exists and is not a git repo: {}",
                repo_path.display()
            )));
        }
    }
    let parent = repo_path.parent().unwrap_or(repo_path);
    git::git_passthrough(parent, &["clone", repo_url, &repo_path.display().to_string()])
}

/// Resolve a source-qualified template ref, cloning the source repository
/// into the template cache when it is missing and `clone_missing_repo` is set.
pub fn resolve_template_source(paths: &Paths, template_ref: &str, clone_missing_repo: bool) -> Result<TemplateSource> {
    let (repo_locator, subdir) = split_source_ref(template_ref)?;
    let RepoLocator { org, repo, clone_url } = parse_repo_locator(&repo_locator)?;

    let local_repo_path = paths.template_root_dir().join(&org).join(&repo);
    if clone_missing_repo {
        ensure_repo_cloned(&clone_url, &local_repo_path)?;
    }
    if !local_repo_path.join(".git").exists() {
        return Err(Error::NotFound(format!(
            "Template source repository is not initialized at {}. Run 'boilersync templates init <repo-url>' or use a source ref to auto-clone.",
            local_repo_path.display()
        )));
    }
    let template_dir = local_repo_path.join(&subdir);
    if !template_dir.exists() {
        return Err(Error::NotFound(format!(
            "Template subdir '{subdir}' not found in template source repo: {}",
            local_repo_path.display()
        )));
    }
    Ok(TemplateSource {
        ref_: format!("{clone_url}#{subdir}"),
        template_dir,
        repo_url: clone_url,
        org,
        repo,
        subdir,
        local_repo_path,
    })
}

/// Resolve the `template` field of a `.boilersync` file. Refs without a
/// source qualifier are legacy names inside the default template repository.
pub fn resolve_source_from_boilersync(
    paths: &Paths,
    template_ref: Option<&str>,
    clone_missing_repo: bool,
) -> Result<TemplateSource> {
    let Some(template_ref) = template_ref.map(str::trim).filter(|value| !value.is_empty()) else {
        return Err(Error::Invalid("Template reference missing in .boilersync metadata.".to_string()));
    };
    let normalized = if template_ref.contains('#') {
        template_ref.to_string()
    } else {
        format!("{DEFAULT_LEGACY_REPO_LOCATOR}#{template_ref}")
    };
    resolve_template_source(paths, &normalized, clone_missing_repo)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_org_repo_and_urls() {
        let parsed = parse_repo_locator("acme/template-kit").unwrap();
        assert_eq!((parsed.org.as_str(), parsed.repo.as_str()), ("acme", "template-kit"));
        assert_eq!(parsed.clone_url, "https://github.com/acme/template-kit.git");
        assert_eq!(
            parse_repo_locator("https://github.com/acme/template-kit").unwrap().clone_url,
            "https://github.com/acme/template-kit.git"
        );
        assert_eq!(
            parse_repo_locator("https://github.com/acme/template-kit.git").unwrap().clone_url,
            "https://github.com/acme/template-kit.git"
        );
    }

    #[test]
    fn applies_known_repo_renames() {
        let parsed = parse_repo_locator("openbase-community/openbase-boilerplate").unwrap();
        assert_eq!(parsed.repo, "templates");
        assert_eq!(parsed.clone_url, "https://github.com/openbase-community/templates.git");
    }

    #[test]
    fn rejects_other_hosts_ssh_and_garbage() {
        assert!(parse_repo_locator("https://gitlab.com/acme/template-kit.git").is_err());
        assert!(parse_repo_locator("git@github.com:acme/template-kit.git").is_err());
        assert!(parse_repo_locator("just-a-name").is_err());
        assert!(parse_repo_locator("").is_err());
    }

    #[test]
    fn rejects_refs_without_subdir_or_escaping_the_repo() {
        assert!(split_source_ref("acme/template-kit").is_err());
        assert!(split_source_ref("acme/template-kit#").is_err());
        assert!(split_source_ref("acme/template-kit#../etc").is_err());
        assert_eq!(split_source_ref("acme/kit#/python/svc/").unwrap().1, "python/svc");
    }
}
