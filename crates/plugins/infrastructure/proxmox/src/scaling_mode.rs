use sea_orm::entity::prelude::*;

/// Scaling mode discriminant stored in the database.
/// Internal-only; not sent over any network boundary.
/// Not `#[non_exhaustive]` — must be exhaustively matched everywhere.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, EnumIter, DeriveActiveEnum)]
#[sea_orm(rs_type = "String", db_type = "Text")]
pub(crate) enum ScalingMode {
    #[default]
    #[sea_orm(string_value = "none")]
    None,
    #[sea_orm(string_value = "absolute")]
    Absolute,
    #[sea_orm(string_value = "delta")]
    Delta,
}

impl ScalingMode {
    pub(crate) fn as_str(self) -> &'static str {
        match self {
            Self::None => "none",
            Self::Absolute => "absolute",
            Self::Delta => "delta",
        }
    }
}

impl std::fmt::Display for ScalingMode {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}

impl std::str::FromStr for ScalingMode {
    type Err = ();

    fn from_str(s: &str) -> std::result::Result<Self, Self::Err> {
        match s {
            "none" => Ok(Self::None),
            "absolute" => Ok(Self::Absolute),
            "delta" => Ok(Self::Delta),
            _ => Err(()),
        }
    }
}
