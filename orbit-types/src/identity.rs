use std::fmt::{Display, Formatter};
use std::str::FromStr;

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum IdentityRole {
    Ceo,
    Architect,
    Reviewer,
    Leader,
    Engineer,
    ProductDesigner,
    Member,
}

impl Display for IdentityRole {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        let value = match self {
            IdentityRole::Ceo => "ceo",
            IdentityRole::Architect => "architect",
            IdentityRole::Reviewer => "reviewer",
            IdentityRole::Leader => "leader",
            IdentityRole::Engineer => "engineer",
            IdentityRole::ProductDesigner => "product-designer",
            IdentityRole::Member => "member",
        };
        write!(f, "{value}")
    }
}

impl FromStr for IdentityRole {
    type Err = String;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        match value.trim().to_ascii_lowercase().as_str() {
            "ceo" => Ok(IdentityRole::Ceo),
            "architect" => Ok(IdentityRole::Architect),
            "reviewer" => Ok(IdentityRole::Reviewer),
            "leader" => Ok(IdentityRole::Leader),
            "engineer" => Ok(IdentityRole::Engineer),
            "product-designer" | "product_designer" => Ok(IdentityRole::ProductDesigner),
            "member" => Ok(IdentityRole::Member),
            other => Err(format!("unknown identity role: {other}")),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ResolvedIdentity {
    pub id: String,
    pub name: String,
    pub role: IdentityRole,
    #[serde(default)]
    pub personality: Vec<(String, String)>,
    #[serde(default)]
    pub behavior: Vec<(String, String)>,
}

#[cfg(test)]
mod tests {
    use super::IdentityRole;

    #[test]
    fn parses_engineer_architect_reviewer_and_product_designer_roles() {
        assert_eq!(
            "engineer".parse::<IdentityRole>().expect("engineer role"),
            IdentityRole::Engineer
        );
        assert_eq!(
            "architect".parse::<IdentityRole>().expect("architect role"),
            IdentityRole::Architect
        );
        assert_eq!(
            "reviewer".parse::<IdentityRole>().expect("reviewer role"),
            IdentityRole::Reviewer
        );
        assert_eq!(
            "product-designer"
                .parse::<IdentityRole>()
                .expect("product-designer role"),
            IdentityRole::ProductDesigner
        );
        assert_eq!(
            "product_designer"
                .parse::<IdentityRole>()
                .expect("product_designer role"),
            IdentityRole::ProductDesigner
        );
    }

    #[test]
    fn display_uses_external_role_strings() {
        assert_eq!(IdentityRole::Engineer.to_string(), "engineer");
        assert_eq!(IdentityRole::Architect.to_string(), "architect");
        assert_eq!(IdentityRole::Reviewer.to_string(), "reviewer");
        assert_eq!(
            IdentityRole::ProductDesigner.to_string(),
            "product-designer"
        );
    }
}
