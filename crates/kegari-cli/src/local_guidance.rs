//! Bounded interactive collection of explicit operator configuration.
//! No provider reads, credential access or effects occur before review/submission.
use super::*;
use kegari_config::{Profile, ProviderKind, Selector, Speed};
use serde::Serialize;
use std::collections::BTreeMap;

impl<R: BufRead, W: Write> SetupPrompt<R, W> {
    fn required(&mut self, label: &str) -> Result<String> {
        let value = self.ask(label)?;
        if value.is_empty() {
            return Err(Error::invalid("required configuration answer is empty"));
        }
        Ok(value)
    }
    fn uuid(&mut self, label: &str) -> Result<Uuid> {
        let id: Uuid = self
            .required(label)?
            .parse()
            .map_err(|_| Error::invalid("configuration identity must be a UUID"))?;
        if id.is_nil() {
            return Err(Error::invalid("configuration UUID must not be nil"));
        }
        Ok(id)
    }
    fn positive(&mut self, label: &str, max: u32) -> Result<u32> {
        let value: u32 = self
            .required(label)?
            .parse()
            .map_err(|_| Error::invalid("configuration count must be a positive integer"))?;
        if value == 0 || value > max {
            return Err(Error::invalid(
                "configuration count is outside the supported bound",
            ));
        }
        Ok(value)
    }
    fn choice(&mut self, label: &str, allowed: &[&str], default: Option<&str>) -> Result<String> {
        let value = self.ask(label)?;
        let selected = if value.is_empty() {
            default.unwrap_or("")
        } else {
            &value
        };
        if !allowed.contains(&selected) {
            return Err(Error::invalid(
                "configuration answer is not one of the listed choices",
            ));
        }
        Ok(selected.into())
    }
    fn list(&mut self, label: &str, allow_empty: bool) -> Result<Vec<String>> {
        let value = self.ask(label)?;
        if value.is_empty() && allow_empty {
            return Ok(Vec::new());
        }
        let values: Vec<_> = value.split(',').map(|v| v.trim().to_owned()).collect();
        if values.len() > 128
            || values.iter().any(|v| v.is_empty())
            || values.iter().collect::<BTreeSet<_>>().len() != values.len()
        {
            return Err(Error::invalid(
                "configuration list must contain bounded, nonempty, unique values",
            ));
        }
        Ok(values)
    }
    fn actor_ids(&mut self, label: &str) -> Result<BTreeSet<Uuid>> {
        self.list(label, false)?
            .into_iter()
            .map(|value| {
                let id: Uuid = value
                    .parse()
                    .map_err(|_| Error::invalid("actor identity must be a UUID"))?;
                if id.is_nil() {
                    return Err(Error::invalid("actor identity must not be nil"));
                }
                Ok(id)
            })
            .collect()
    }
    fn selector(&mut self, label: &str) -> Result<Selector> {
        let values = self.list(label, false)?;
        let selector = if values == ["all"] {
            Selector::All("all".into())
        } else {
            Selector::Ids(values)
        };
        selector.validate()?;
        Ok(selector)
    }
    fn profile(&mut self, label: &str) -> Result<Profile> {
        writeln!(self.output,"{label}: select the actual Codex model and settings; provider support is checked at submission.").map_err(|e|Error::io("write profile guidance",e))?;
        let model = self.required("Exact model ID: ")?;
        let reasoning =
            self.required("Reasoning (none/minimal/low/medium/high/xhigh/max/ultra): ")?;
        let speed = self.choice(
            "Speed (standard/fast) [standard]: ",
            &["standard", "fast"],
            Some("standard"),
        )?;
        let profile = Profile {
            tool: "codex".into(),
            model,
            reasoning,
            speed: if speed == "fast" {
                Speed::Fast
            } else {
                Speed::Standard
            },
        };
        profile.validate()?;
        Ok(profile)
    }
    fn review<T: Serialize>(&mut self, value: &T, explanation: &str) -> Result<String> {
        let document = toml::to_string_pretty(value)
            .map_err(|_| Error::invalid("configuration could not be encoded"))?;
        writeln!(
            self.output,
            "\nReview the complete configuration:\n{document}\n{explanation}"
        )
        .map_err(|e| Error::io("write configuration review", e))?;
        if self.ask("Type submit to send this reviewed configuration, or /cancel: ")? != "submit" {
            return Err(setup_cancelled());
        }
        Ok(document)
    }
}

pub fn guided_connection(
    kind: ProviderKind,
    input: impl BufRead,
    output: impl Write,
) -> Result<String> {
    let mut prompt = SetupPrompt {
        input,
        output,
        bytes_read: 0,
    };
    writeln!(prompt.output,"Connection setup. Enter /cancel at any prompt. Do not enter access tokens, passwords or authentication files. Account IDs and references are verified by the ordinary configuration transaction; this form does not authenticate or create a login.").map_err(|e|Error::io("write connection guidance",e))?;
    let account_id = prompt.required("Exact provider account ID (not a display name): ")?;
    let mut connection: Connection =
        serde_json::from_value(json!({"kind":kind,"account_id":account_id}))
            .map_err(|_| Error::invalid("connection defaults are invalid"))?;
    if kind == ProviderKind::Codex {
        connection.auth_home = Some(
            prompt.required("Isolated Codex authentication home (absolute path or ~/ path): ")?,
        );
        let executable = prompt.ask("Codex executable path or command [codex]: ")?;
        if !executable.is_empty() {
            connection.argv[0] = executable;
        }
        connection.max_agents =
            Some(prompt.positive("Maximum simultaneous agents for this account: ", u32::MAX)?);
    } else {
        let default = if kind == ProviderKind::Linear {
            "https://api.linear.app/graphql"
        } else {
            "https://api.github.com/"
        };
        let endpoint = prompt.ask(&format!("Provider API endpoint [{default}]: "))?;
        let endpoint = if endpoint.is_empty() {
            default.to_owned()
        } else {
            endpoint
        };
        let url = url::Url::parse(&endpoint)
            .map_err(|_| Error::invalid("provider endpoint must be an absolute HTTPS URL"))?;
        if url.scheme() != "https"
            || url.host_str().is_none()
            || !url.username().is_empty()
            || url.password().is_some()
            || url.query().is_some()
            || url.fragment().is_some()
        {
            return Err(Error::invalid(
                "provider endpoint must be credential-free HTTPS without query or fragment",
            ));
        }
        connection.endpoint = Some(endpoint);
        let source = prompt.choice(
            "Credential reference source (environment/keychain): ",
            &["environment", "keychain"],
            None,
        )?;
        let name = prompt.required(if source == "environment" {
            "Environment variable NAME only (never its value): "
        } else {
            "Existing Keychain reference NAME only (never its secret): "
        })?;
        if source == "environment"
            && !name
                .bytes()
                .enumerate()
                .all(|(i, b)| b == b'_' || b.is_ascii_alphabetic() || i > 0 && b.is_ascii_digit())
        {
            return Err(Error::invalid(
                "environment reference must be a variable name, not a secret value",
            ));
        }
        connection.credential_ref = Some(if source == "environment" {
            format!("${name}")
        } else {
            format!("keychain:{name}")
        });
    }
    prompt.review(&connection,"This submits the existing connection-add transaction. Provider identity, endpoint boundary, credentials and supported Codex metadata are checked there. No workspace, repository write scope or agent work is enabled by this form. For custom argument arrays use connection add --from with the same typed schema.")
}

pub fn guided_workspace(input: impl BufRead, output: impl Write) -> Result<String> {
    let mut prompt = SetupPrompt {
        input,
        output,
        bytes_read: 0,
    };
    writeln!(prompt.output,"Workspace setup. Enter /cancel at any prompt. Supply actual provider IDs and explicit authority; this form does not discover or invent identities. No command catalog or environment manifest is required. Hosted delivery policy may be configured later; its dependent delivery gates remain blocked.").map_err(|e|Error::io("write workspace guidance",e))?;
    let linear_workspace_id = prompt.uuid("Linear workspace UUID: ")?;
    let linear_connection = prompt.required("Existing Linear connection name: ")?;
    let github_connection = prompt.required("Existing GitHub connection name: ")?;
    let codex_connection = prompt.required("Existing Codex connection name: ")?;
    let workflow =
        prompt.ask("Explicit workflow path, or blank for installed generic guidance: ")?;
    let teams = prompt.selector("Linear team IDs, comma-separated, or explicit all: ")?;
    let projects = prompt.selector("Linear project IDs, comma-separated, or explicit all: ")?;
    let users =
        prompt.selector("Linear creator/assignee IDs, comma-separated, or explicit all: ")?;
    let authorization_label_id = prompt.uuid("Linear authorization-label UUID: ")?;
    let team_count = prompt.positive(
        "Number of teams with explicit lifecycle state mappings: ",
        128,
    )?;
    let mut states = BTreeMap::new();
    for _ in 0..team_count {
        let team = prompt.required("Team ID for this state mapping: ")?;
        if states.contains_key(&team) {
            return Err(Error::invalid("team state mapping repeated"));
        }
        let mapping = kegari_config::StateMapping {
            prepared: prompt.uuid("Prepared state UUID: ")?,
            running: prompt.uuid("Running state UUID: ")?,
            blocked: prompt.uuid("Blocked state UUID: ")?,
            child_done: prompt.uuid("Child done state UUID: ")?,
            parent_accepted: prompt.uuid("Parent accepted state UUID: ")?,
            cancelled: prompt.uuid("Cancelled state UUID: ")?,
        };
        states.insert(team, mapping);
    }
    let authorizers =
        prompt.actor_ids("Allowed authorizer Linear user UUIDs, comma-separated: ")?;
    let correctors = prompt.actor_ids("Allowed corrector Linear user UUIDs, comma-separated: ")?;
    let acceptors = prompt.actor_ids("Allowed acceptor Linear user UUIDs, comma-separated: ")?;
    let coordination = prompt.profile("Coordination profile")?;
    let child_profile = prompt.profile("Initial child routing profile")?;
    let tool_label = prompt.uuid("Child Codex tool routing-label UUID: ")?;
    let model_label = prompt.uuid("Child model routing-label UUID: ")?;
    let reasoning_label = prompt.uuid("Child reasoning routing-label UUID: ")?;
    let speed_label = prompt.uuid("Child speed routing-label UUID: ")?;
    if BTreeSet::from([tool_label, model_label, reasoning_label, speed_label]).len() != 4 {
        return Err(Error::invalid("routing label UUIDs must be distinct"));
    }
    let max_agents = prompt.positive("Workspace maximum simultaneous agents: ", u32::MAX)?;
    let independent = prompt.choice(
        "Enable independent PR review (yes/no) [no]: ",
        &["yes", "no"],
        Some("no"),
    )? == "yes";
    let (independent_review, pr_authors) = if independent {
        (
            Some(prompt.profile("Independent PR review profile")?),
            prompt.list(
                "Allowed GitHub PR-author numeric IDs, comma-separated: ",
                false,
            )?,
        )
    } else {
        (None, vec![])
    };
    let release = prompt.choice(
        "Release mode (manual/automatic) [manual]: ",
        &["manual", "automatic"],
        Some("manual"),
    )?;
    let spacing = prompt.ask("Minimum release spacing in seconds [0]: ")?;
    let spacing: u64 = if spacing.is_empty() {
        0
    } else {
        spacing
            .parse()
            .map_err(|_| Error::invalid("release spacing must be nonnegative seconds"))?
    };
    let repository_count = prompt.positive("Number of explicitly allowed repositories: ", 128)?;
    let mut repositories = BTreeMap::<String, kegari_config::Repository>::new();
    for _ in 0..repository_count {
        let id = prompt.required("Canonical GitHub numeric repository ID: ")?;
        if id.parse::<u64>().ok().is_none_or(|id| id == 0) || repositories.contains_key(&id) {
            return Err(Error::invalid("repository ID must be positive and unique"));
        }
        let source = prompt
            .required("Local repository source path (existing or unborn Git repository): ")?;
        let hostname = prompt.ask("Repository hostname [github.com]: ")?;
        let main = prompt.ask("Production branch name [main]: ")?;
        let staging = prompt.ask("Staging branch name [staging]: ")?;
        let mut value = json!({"source":source});
        for (key, text) in [("hostname", hostname), ("main", main), ("staging", staging)] {
            if !text.is_empty() {
                value[key] = json!(text)
            }
        }
        let repository = serde_json::from_value(value)
            .map_err(|_| Error::invalid("repository fields do not match configuration schema"))?;
        repositories.insert(id, repository);
    }
    let workspace = Workspace {
        id: Uuid::new_v4(),
        linear_workspace_id,
        linear_connection,
        github_connection,
        codex_connection,
        workflow: if workflow.is_empty() {
            None
        } else {
            Some(workflow)
        },
        linear: kegari_config::LinearSelection {
            teams,
            projects,
            users,
            authorization_label_id,
            states,
        },
        github: kegari_config::GithubSelection {
            organizations: vec![],
            repositories: repositories.keys().cloned().collect(),
            pr_authors,
        },
        independent_review: kegari_config::IndependentReview {
            enabled: independent,
        },
        actors: kegari_config::Actors {
            authorizers,
            correctors,
            acceptors,
        },
        profiles: kegari_config::Profiles {
            coordination,
            independent_review,
        },
        routing_labels: kegari_config::RoutingLabels {
            tool: BTreeMap::from([(tool_label, "codex".into())]),
            model: BTreeMap::from([(model_label, child_profile.model)]),
            reasoning: BTreeMap::from([(reasoning_label, child_profile.reasoning)]),
            speed: BTreeMap::from([(speed_label, child_profile.speed)]),
        },
        max_agents,
        max_agents_by_state: BTreeMap::new(),
        release: kegari_config::ReleaseConfig {
            mode: if release == "automatic" {
                kegari_config::ReleaseMode::Automatic
            } else {
                kegari_config::ReleaseMode::Manual
            },
            spacing_seconds: spacing,
        },
        repositories,
        test_resources: BTreeMap::new(),
        evaluation: None,
    };
    prompt.review(&workspace,"This draft grants only the explicit repository/actor scope shown. It starts with one child routing profile; add further mappings through workspace configure. Delivery CI, production CI, no-change validation and deployment policy are omitted, not waived or considered passing. The existing configuration transaction validates connection references, provider identity, selectors and paths before saving. No service or agent is started by this form.")
}
