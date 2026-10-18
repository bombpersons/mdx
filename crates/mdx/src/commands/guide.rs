//! Built-in workflow guidance, available without a checkout or network.
use crate::envelope::{Coverage, Envelope, Failure};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Deserialize, Serialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum Topic {
    /// What mdx is for and how an investigation usually goes.
    #[default]
    Overview,
    /// Building a native port as an engine plus a game, with template.create.
    Port,
}

impl Topic {
    const ALL: [Self; 2] = [Self::Overview, Self::Port];

    fn name(self) -> &'static str {
        match self {
            Self::Overview => "overview",
            Self::Port => "port",
        }
    }

    fn title(self) -> &'static str {
        match self {
            Self::Overview => "Investigate a cartridge with mdx",
            Self::Port => "Port a game as an engine plus the game",
        }
    }

    fn text(self) -> &'static str {
        match self {
            Self::Overview => include_str!("guide/overview.md"),
            Self::Port => include_str!("guide/port.md"),
        }
    }
}

#[derive(Debug, Deserialize, Serialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct GuideArgs {
    #[serde(default)]
    pub topic: Topic,
}

#[derive(Debug, Serialize, JsonSchema)]
pub struct GuideData {
    pub topic: Topic,
    pub title: String,
    /// The guidance, as Markdown.
    pub text: String,
    /// Every topic, as `name: title`.
    pub topics: Vec<String>,
}

pub fn run(args: GuideArgs) -> Result<Envelope<GuideData>, Failure> {
    Ok(Envelope::ok(
        "mdx.guide",
        None,
        Coverage::complete(vec!["static guidance embedded in this mdx build; no files read".into()]),
        GuideData {
            topic: args.topic,
            title: args.topic.title().into(),
            text: args.topic.text().into(),
            topics: Topic::ALL.iter().map(|t| format!("{}: {}", t.name(), t.title())).collect(),
        },
    ))
}
