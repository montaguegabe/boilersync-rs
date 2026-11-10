//! Project name conventions: snake_case, PascalCase, kebab-case, camelCase and
//! a pretty display name.

/// The different naming conventions of a project name.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProjectNames {
    /// `my_project_name`
    pub snake: String,
    /// `MyProjectName`
    pub pascal: String,
    /// `my-project-name`
    pub kebab: String,
    /// `myProjectName`
    pub camel: String,
    /// `My Project Name`
    pub pretty: String,
}

/// Python's `str.capitalize()`: first character upper-cased, the rest lower-cased.
fn capitalize(word: &str) -> String {
    let mut chars = word.chars();
    match chars.next() {
        Some(first) => first.to_uppercase().chain(chars.flat_map(|c| c.to_lowercase())).collect(),
        None => String::new(),
    }
}

/// Convert any naming convention to snake_case.
pub fn normalize_to_snake(name: &str) -> String {
    if name.contains('-') {
        return name.replace('-', "_").to_lowercase();
    }
    // Insert an underscore before every upper-case letter except the first character.
    let mut snake = String::with_capacity(name.len() + 4);
    for (index, ch) in name.chars().enumerate() {
        if index > 0 && ch.is_uppercase() {
            snake.push('_');
        }
        snake.extend(ch.to_lowercase());
    }
    snake
}

/// Infer a project name from a directory name, dropping a `-workspace` /
/// `_workspace` suffix.
pub fn default_project_snake_from_directory_name(directory_name: &str) -> String {
    let snake = normalize_to_snake(directory_name);
    match snake.strip_suffix("_workspace") {
        Some(stem) if !stem.is_empty() => stem.to_string(),
        _ => snake,
    }
}

pub fn snake_to_pascal(snake: &str) -> String {
    snake.split('_').map(capitalize).collect()
}

pub fn snake_to_kebab(snake: &str) -> String {
    snake.replace('_', "-")
}

pub fn snake_to_camel(snake: &str) -> String {
    let mut words = snake.split('_');
    let mut camel = words.next().unwrap_or("").to_string();
    camel.extend(words.map(capitalize));
    camel
}

pub fn snake_to_pretty(snake: &str) -> String {
    snake.split('_').map(capitalize).collect::<Vec<_>>().join(" ")
}

/// All naming conventions from a snake_case name and an optional pretty name.
pub fn create_project_names(snake: &str, pretty: Option<&str>) -> ProjectNames {
    ProjectNames {
        snake: snake.to_string(),
        pascal: snake_to_pascal(snake),
        kebab: snake_to_kebab(snake),
        camel: snake_to_camel(snake),
        pretty: pretty.map(str::to_string).unwrap_or_else(|| snake_to_pretty(snake)),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn normalizes_every_convention_to_snake() {
        assert_eq!(normalize_to_snake("my-project"), "my_project");
        assert_eq!(normalize_to_snake("MyProject"), "my_project");
        assert_eq!(normalize_to_snake("myProject"), "my_project");
        assert_eq!(normalize_to_snake("my_project"), "my_project");
    }

    #[test]
    fn strips_workspace_suffix_from_directory_names() {
        assert_eq!(default_project_snake_from_directory_name("woo-score-workspace"), "woo_score");
        assert_eq!(default_project_snake_from_directory_name("woo-score"), "woo_score");
        assert_eq!(default_project_snake_from_directory_name("workspace"), "workspace");
    }

    #[test]
    fn derives_all_conventions() {
        let names = create_project_names("my_project", None);
        assert_eq!(names.pascal, "MyProject");
        assert_eq!(names.kebab, "my-project");
        assert_eq!(names.camel, "myProject");
        assert_eq!(names.pretty, "My Project");
        assert_eq!(create_project_names("x", Some("Custom")).pretty, "Custom");
    }
}
