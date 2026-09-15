use serde::Serialize;

/// Runtime integration status for an inherited module.
#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct RuntimeModuleStatus {
    /// Module name.
    pub name: &'static str,
    /// Whether the module is compiled into this build.
    pub enabled: bool,
    /// Intended role in OpenCompany.
    pub role: &'static str,
    /// Local source location.
    pub path: &'static str,
}

impl RuntimeModuleStatus {
    /// Returns the status of all inherited runtime modules.
    pub fn all() -> Vec<Self> {
        vec![
            Self {
                name: "tinyagents",
                // The checkout is still `tinyagents`, but it is a workspace now
                // and this crate links two of its members: the harness, and
                // `tinyinference` (vendored under it) for the model layer. The
                // graph/registry/RLM members are openhuman's, not ours.
                enabled: cfg!(feature = "tinyagents-harness"),
                role: "agent harness and inference model layer",
                path: "vendor/openhuman/vendor/tinyagents",
            },
            Self {
                name: "openhuman",
                enabled: true,
                role: "OpenHuman checkout launched through Cargo",
                path: "vendor/openhuman",
            },
        ]
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn all_known_modules_are_reported() {
        let modules = RuntimeModuleStatus::all();
        let names: Vec<_> = modules.iter().map(|module| module.name).collect();

        assert_eq!(names, vec!["tinyagents", "openhuman"]);
        assert_eq!(modules[0].path, "vendor/openhuman/vendor/tinyagents");
    }
}
