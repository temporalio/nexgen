use std::fmt;

#[derive(Copy, Clone, Debug, Eq, PartialEq, Hash, Ord, PartialOrd, serde::Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Language {
    Dotnet,
    Go,
    Java,
    Python,
    Ruby,
    TypeScript,
}

impl Language {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Dotnet => "dotnet",
            Self::Go => "go",
            Self::Java => "java",
            Self::Python => "python",
            Self::Ruby => "ruby",
            Self::TypeScript => "typescript",
        }
    }
}

impl fmt::Display for Language {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}
