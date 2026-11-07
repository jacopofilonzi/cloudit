//! [`ExternalIdentity`]: who the provider says the user is, normalized the same way for
//! every protocol, and the group rules applied to it.

use serde_json::Value;

use crate::validate;

/// The verified result of a provider sign-in.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ExternalIdentity {
    /// Stable account id at the provider (OIDC `sub`, GitHub/Discord user id).
    pub subject: String,
    /// Normalized (`validate::email`); `None` if missing or malformed.
    pub email: Option<String>,
    /// The provider vouches for `email` (a missing claim counts as `false`).
    pub email_verified: bool,
    /// Suggested display name (never empty, at most 100 characters).
    pub display_name: String,
    /// Groups from the groups claim; `None` when the provider sent none (or has no
    /// groups at all).
    pub groups: Option<Vec<String>>,
}

impl ExternalIdentity {
    /// Builds an identity from raw values, normalizing email and name.
    #[must_use]
    pub fn new(
        subject: String,
        email: Option<&str>,
        email_verified: bool,
        names: &[Option<&str>],
        groups: Option<Vec<String>>,
    ) -> Self {
        let email = email.and_then(|e| validate::email(e, "email").ok());
        let email_verified = email_verified && email.is_some();
        let display_name = names
            .iter()
            .flatten()
            .find_map(|n| clean_name(n))
            .or_else(|| {
                email
                    .as_deref()
                    .and_then(|e| e.split('@').next())
                    .and_then(clean_name)
            })
            .unwrap_or_else(|| "User".to_owned());
        Self {
            subject,
            email,
            email_verified,
            display_name,
            groups,
        }
    }

    /// `true` if the identity is in at least one of `groups`.
    #[must_use]
    pub fn in_any(&self, groups: &[String]) -> bool {
        self.groups
            .as_deref()
            .unwrap_or_default()
            .iter()
            .any(|g| groups.contains(g))
    }

    /// The provider's `allowed_groups` rule: empty means everyone.
    #[must_use]
    pub fn allowed(&self, allowed_groups: &[String]) -> bool {
        allowed_groups.is_empty() || self.in_any(allowed_groups)
    }

    /// Group-derived admin after a sign-in through a provider with `admin_groups`:
    /// non-empty `admin_groups` and membership in one of them.
    #[must_use]
    pub fn admin_from_groups(&self, admin_groups: &[String]) -> bool {
        !admin_groups.is_empty() && self.in_any(admin_groups)
    }
}

/// A usable display name: trimmed, control characters removed, at most 100
/// characters; `None` if nothing is left.
fn clean_name(raw: &str) -> Option<String> {
    let cleaned: String = raw
        .chars()
        .filter(|c| !c.is_control())
        .collect::<String>()
        .trim()
        .chars()
        .take(validate::MAX_NAME_CHARS)
        .collect();
    let cleaned = cleaned.trim().to_owned();
    (!cleaned.is_empty()).then_some(cleaned)
}

/// Groups from a claim value: an array of strings (other elements ignored) or a single
/// string; any other type is no groups. `None` when the claim is absent.
#[must_use]
pub fn groups_from(value: Option<&Value>) -> Option<Vec<String>> {
    match value? {
        Value::Array(items) => Some(
            items
                .iter()
                .filter_map(Value::as_str)
                .map(str::to_owned)
                .collect(),
        ),
        Value::String(single) => Some(vec![single.clone()]),
        Value::Null => None,
        _ => Some(Vec::new()),
    }
}

/// A boolean claim that may come as `true` / `"true"`; anything else is `false`.
#[must_use]
pub fn bool_claim(value: Option<&Value>) -> bool {
    match value {
        Some(Value::Bool(b)) => *b,
        Some(Value::String(s)) => s.eq_ignore_ascii_case("true"),
        _ => false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn normalizes_email_and_name() {
        let id = ExternalIdentity::new(
            "1".to_owned(),
            Some(" Ada@Example.COM "),
            true,
            &[None, Some("  \u{7}  "), Some(" Ada Lovelace ")],
            None,
        );
        assert_eq!(id.email.as_deref(), Some("ada@example.com"));
        assert!(id.email_verified);
        assert_eq!(id.display_name, "Ada Lovelace");
        let id = ExternalIdentity::new("1".to_owned(), Some("not-an-email"), true, &[], None);
        assert_eq!(id.email, None);
        assert!(!id.email_verified, "no email, nothing verified");
        assert_eq!(id.display_name, "User");
        let id = ExternalIdentity::new("1".to_owned(), Some("grace@example.com"), false, &[], None);
        assert_eq!(id.display_name, "grace");
        let long = "x".repeat(300);
        let id = ExternalIdentity::new("1".to_owned(), None, false, &[Some(&long)], None);
        assert_eq!(id.display_name.chars().count(), 100);
    }

    #[test]
    fn groups_claim_shapes() {
        assert_eq!(groups_from(None), None);
        assert_eq!(groups_from(Some(&json!(null))), None);
        assert_eq!(
            groups_from(Some(&json!(["a", 1, "b", {"x": 1}]))),
            Some(vec!["a".to_owned(), "b".to_owned()])
        );
        assert_eq!(
            groups_from(Some(&json!("admins"))),
            Some(vec!["admins".to_owned()])
        );
        assert_eq!(groups_from(Some(&json!(42))), Some(vec![]));
        assert_eq!(groups_from(Some(&json!({"a": 1}))), Some(vec![]));
    }

    #[test]
    fn group_rules() {
        let id = ExternalIdentity::new(
            "1".to_owned(),
            None,
            false,
            &[],
            Some(vec!["staff".to_owned()]),
        );
        let staff = vec!["staff".to_owned()];
        let admins = vec!["admins".to_owned()];
        assert!(id.allowed(&[]));
        assert!(id.allowed(&staff));
        assert!(!id.allowed(&admins));
        assert!(id.admin_from_groups(&staff));
        assert!(!id.admin_from_groups(&admins));
        assert!(!id.admin_from_groups(&[]));
        let none = ExternalIdentity::new("1".to_owned(), None, false, &[], None);
        assert!(!none.allowed(&staff));
        assert!(none.allowed(&[]));
    }

    #[test]
    fn boolean_claims() {
        assert!(bool_claim(Some(&json!(true))));
        assert!(bool_claim(Some(&json!("TRUE"))));
        assert!(!bool_claim(Some(&json!("yes"))));
        assert!(!bool_claim(Some(&json!(1))));
        assert!(!bool_claim(None));
    }
}
