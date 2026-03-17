/// Convert a software item name to a Dashboard Icons slug.
///
/// Rules (matching Dashboard Icons naming convention):
/// - Lowercase
/// - Replace whitespace and underscores with hyphens
/// - Collapse consecutive hyphens
/// - Strip leading/trailing hyphens
/// - Remove characters that are not alphanumeric or hyphens
pub(crate) fn slugify(name: &str) -> String {
    let mut result = String::with_capacity(name.len());
    let mut prev_hyphen = true; // suppress leading hyphens

    for ch in name.chars() {
        if ch.is_ascii_alphanumeric() {
            result.push(ch.to_ascii_lowercase());
            prev_hyphen = false;
        } else if (ch == ' ' || ch == '_' || ch == '-') && !prev_hyphen {
            result.push('-');
            prev_hyphen = true;
        }
        // All other characters are silently dropped.
    }

    // Strip trailing hyphen.
    if result.ends_with('-') {
        result.pop();
    }

    result
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn simple_names() {
        assert_eq!(slugify("Nginx"), "nginx");
        assert_eq!(slugify("Redis"), "redis");
        assert_eq!(slugify("Grafana"), "grafana");
        assert_eq!(slugify("Prometheus"), "prometheus");
    }

    #[test]
    fn names_with_spaces() {
        assert_eq!(slugify("Home Assistant"), "home-assistant");
        assert_eq!(slugify("Pi hole"), "pi-hole");
    }

    #[test]
    fn names_with_special_characters() {
        assert_eq!(slugify("Node.js"), "nodejs");
        assert_eq!(slugify("ASP.NET Core"), "aspnet-core");
        assert_eq!(slugify("n8n"), "n8n");
    }

    #[test]
    fn names_with_underscores() {
        assert_eq!(slugify("my_app"), "my-app");
    }

    #[test]
    fn mixed_case() {
        assert_eq!(slugify("AdGuard Home"), "adguard-home");
        assert_eq!(slugify("MariaDB"), "mariadb");
    }

    #[test]
    fn leading_trailing_hyphens_stripped() {
        assert_eq!(slugify("-leading"), "leading");
        assert_eq!(slugify("trailing-"), "trailing");
        assert_eq!(slugify("--double--"), "double");
    }

    #[test]
    fn consecutive_separators_collapsed() {
        assert_eq!(slugify("a  b"), "a-b");
        assert_eq!(slugify("a--b"), "a-b");
        assert_eq!(slugify("a - b"), "a-b");
    }

    #[test]
    fn empty_and_whitespace() {
        assert_eq!(slugify(""), "");
        assert_eq!(slugify("   "), "");
    }

    #[test]
    fn numbers_preserved() {
        assert_eq!(slugify("PostgreSQL 16"), "postgresql-16");
        assert_eq!(slugify("3CX"), "3cx");
    }
}
