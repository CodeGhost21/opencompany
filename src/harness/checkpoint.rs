//! Automatic Git checkpoints for an agent's private filesystem workspace.
//!
//! The feature is deliberately a tool decorator: file tools, patches, shell
//! redirects, downloads, and future workspace-writing tools all pass the same
//! after-call boundary. A call that changed nothing produces no commit, and a
//! Git failure is logged without replacing the tool's real result.

use std::path::{Path, PathBuf};
use std::process::{Command, ExitStatus};
use std::sync::Arc;

use async_trait::async_trait;
use serde_json::Value;

use oh::agent::tool_policy::GeneratedToolRuntimeContext;
use oh::tools::traits::{
    PermissionLevel, Tool, ToolCallOptions, ToolCategory, ToolResult, ToolScope, ToolTimeout,
};
use openhuman_core::openhuman as oh;

use crate::store::fs::path_lock;

const CHECKPOINT_AUTHOR_NAME: &str = "OpenCompany Workspace";
const CHECKPOINT_AUTHOR_EMAIL: &str = "workspace@opencompany.local";

/// A Git repository whose working tree is one agent workspace.
#[derive(Clone, Debug)]
pub(crate) struct WorkspaceCheckpointer {
    workspace: PathBuf,
    git_dir: PathBuf,
}

impl WorkspaceCheckpointer {
    /// Initializes (or reopens) the workspace repository and records a baseline.
    ///
    /// New repositories keep their object database beside the workspace at
    /// `workspace.git`; `workspace/.git` is only Git's small pointer file. This
    /// keeps history out of the files agents enumerate and publish while still
    /// allowing ordinary `git` commands run inside the workspace to discover it.
    pub(crate) fn initialize(workspace: &Path) -> anyhow::Result<Self> {
        std::fs::create_dir_all(workspace)?;
        let out_of_band = workspace.with_extension("git");
        let git_dir = if workspace.join(".git").exists() {
            discover_git_dir(workspace).unwrap_or_else(|_| out_of_band.clone())
        } else {
            out_of_band.clone()
        };

        if !git_dir.join("HEAD").is_file() {
            let status = Command::new("git")
                .args(["init", "--quiet", "--initial-branch=checkpoints"])
                .arg("--separate-git-dir")
                .arg(&git_dir)
                .arg(workspace)
                .status()?;
            require_success(status, "git init")?;
        } else if !workspace.join(".git").exists() && git_dir == out_of_band {
            // The agent may remove hidden files from its working tree. The
            // explicit-dir checkpointer still works, but ordinary Git commands
            // inside the workspace would stop discovering the repository.
            std::fs::write(
                workspace.join(".git"),
                format!("gitdir: {}\n", git_dir.display()),
            )?;
        }

        let checkpointer = Self {
            workspace: workspace.to_path_buf(),
            git_dir,
        };
        checkpointer.checkpoint_unlocked("initialize workspace", true)?;
        Ok(checkpointer)
    }

    /// Records current workspace changes. Failures are returned for the caller
    /// to log, never folded into the tool result.
    async fn checkpoint(&self, tool_name: &str) -> anyhow::Result<()> {
        let lock = path_lock(&self.git_dir);
        let _guard = lock.lock().await;
        let this = self.clone();
        let message = format!("after {tool_name}");
        tokio::task::spawn_blocking(move || this.checkpoint_unlocked(&message, false)).await??;
        Ok(())
    }

    fn checkpoint_unlocked(&self, message: &str, allow_empty_initial: bool) -> anyhow::Result<()> {
        require_success(self.git(["add", "--all"])?.status, "git add")?;

        let diff = self.git(["diff", "--cached", "--quiet"])?.status;
        let has_changes = match diff.code() {
            Some(0) => false,
            Some(1) => true,
            _ => anyhow::bail!("git diff --cached failed with {diff}"),
        };
        let has_head = self
            .git(["rev-parse", "--verify", "HEAD"])?
            .status
            .success();
        if !has_changes && (has_head || !allow_empty_initial) {
            return Ok(());
        }

        let mut command = self.base_git();
        command.args(["commit", "--quiet"]);
        if !has_changes {
            command.arg("--allow-empty");
        }
        let output = command
            .arg("-m")
            .arg(format!("checkpoint: {message}"))
            .output()?;
        if !output.status.success() {
            anyhow::bail!(
                "git commit failed: {}",
                String::from_utf8_lossy(&output.stderr).trim()
            );
        }
        Ok(())
    }

    fn base_git(&self) -> Command {
        let mut command = Command::new("git");
        command
            .arg(format!("--git-dir={}", self.git_dir.display()))
            .arg(format!("--work-tree={}", self.workspace.display()))
            .args(["-c", &format!("user.name={CHECKPOINT_AUTHOR_NAME}")])
            .args(["-c", &format!("user.email={CHECKPOINT_AUTHOR_EMAIL}")]);
        command
    }

    fn git<const N: usize>(&self, args: [&str; N]) -> anyhow::Result<std::process::Output> {
        Ok(self.base_git().args(args).output()?)
    }
}

fn discover_git_dir(workspace: &Path) -> anyhow::Result<PathBuf> {
    let output = Command::new("git")
        .args(["rev-parse", "--absolute-git-dir"])
        .current_dir(workspace)
        .output()?;
    if !output.status.success() {
        anyhow::bail!("could not discover the existing workspace Git directory");
    }
    Ok(PathBuf::from(String::from_utf8(output.stdout)?.trim()))
}

fn require_success(status: ExitStatus, operation: &str) -> anyhow::Result<()> {
    if status.success() {
        Ok(())
    } else {
        anyhow::bail!("{operation} failed with {status}")
    }
}

/// Wraps a tool and checkpoints the workspace after every completed call.
///
/// Every tool is wrapped rather than maintaining a fragile list of writers.
/// Read-only and external tools pay only an unchanged-tree check, while shell
/// redirects and newly-added writers cannot bypass checkpointing accidentally.
pub(crate) struct CheckpointingTool {
    inner: Box<dyn Tool>,
    checkpointer: Arc<WorkspaceCheckpointer>,
}

impl CheckpointingTool {
    pub(crate) fn wrap_all(
        tools: Vec<Box<dyn Tool>>,
        checkpointer: WorkspaceCheckpointer,
    ) -> Vec<Box<dyn Tool>> {
        let checkpointer = Arc::new(checkpointer);
        tools
            .into_iter()
            .map(|inner| {
                Box::new(Self {
                    inner,
                    checkpointer: checkpointer.clone(),
                }) as Box<dyn Tool>
            })
            .collect()
    }

    async fn checkpoint_after<T>(&self, result: anyhow::Result<T>) -> anyhow::Result<T> {
        if let Err(error) = self.checkpointer.checkpoint(self.inner.name()).await {
            tracing::warn!(
                tool = self.inner.name(),
                workspace = %self.checkpointer.workspace.display(),
                %error,
                "[workspace-checkpoint] Git checkpoint failed; preserving the tool result"
            );
        }
        result
    }
}

#[async_trait]
impl Tool for CheckpointingTool {
    fn name(&self) -> &str {
        self.inner.name()
    }
    fn description(&self) -> &str {
        self.inner.description()
    }
    fn parameters_schema(&self) -> Value {
        self.inner.parameters_schema()
    }
    fn supports_markdown(&self) -> bool {
        self.inner.supports_markdown()
    }
    fn permission_level(&self) -> PermissionLevel {
        self.inner.permission_level()
    }
    fn permission_level_with_args(&self, args: &Value) -> PermissionLevel {
        self.inner.permission_level_with_args(args)
    }
    fn scope(&self) -> ToolScope {
        self.inner.scope()
    }
    fn category(&self) -> ToolCategory {
        self.inner.category()
    }
    fn is_concurrency_safe(&self, args: &Value) -> bool {
        self.inner.is_concurrency_safe(args)
    }
    fn external_effect(&self) -> bool {
        self.inner.external_effect()
    }
    fn external_effect_with_args(&self, args: &Value) -> bool {
        self.inner.external_effect_with_args(args)
    }
    fn generated_runtime_context(&self, args: &Value) -> Option<GeneratedToolRuntimeContext> {
        self.inner.generated_runtime_context(args)
    }
    fn max_result_size_chars(&self) -> Option<usize> {
        self.inner.max_result_size_chars()
    }
    fn timeout_policy(&self, args: &Value) -> ToolTimeout {
        self.inner.timeout_policy(args)
    }
    fn display_label(&self, args: &Value) -> Option<String> {
        self.inner.display_label(args)
    }
    fn display_detail(&self, args: &Value) -> Option<String> {
        self.inner.display_detail(args)
    }

    async fn execute(&self, args: Value) -> anyhow::Result<ToolResult> {
        let result = self.inner.execute(args).await;
        self.checkpoint_after(result).await
    }

    async fn execute_with_options(
        &self,
        args: Value,
        options: ToolCallOptions,
    ) -> anyhow::Result<ToolResult> {
        let result = self.inner.execute_with_options(args, options).await;
        self.checkpoint_after(result).await
    }

    async fn execute_with_context(
        &self,
        args: Value,
        options: ToolCallOptions,
        context: Option<&tinyagents::harness::tool::ToolExecutionContext>,
    ) -> anyhow::Result<ToolResult> {
        let result = self
            .inner
            .execute_with_context(args, options, context)
            .await;
        self.checkpoint_after(result).await
    }
}

#[cfg(test)]
mod test {
    use super::*;
    use serde_json::json;
    use tempfile::TempDir;

    struct WriteTool(PathBuf);

    #[async_trait]
    impl Tool for WriteTool {
        fn name(&self) -> &str {
            "write_fixture"
        }
        fn description(&self) -> &str {
            "writes a fixture"
        }
        fn parameters_schema(&self) -> Value {
            json!({"type": "object"})
        }
        async fn execute(&self, args: Value) -> anyhow::Result<ToolResult> {
            std::fs::write(&self.0, args["body"].as_str().unwrap_or_default())?;
            Ok(ToolResult::success("written"))
        }
    }

    fn log(workspace: &Path) -> String {
        String::from_utf8(
            Command::new("git")
                .args(["log", "--format=%s"])
                .current_dir(workspace)
                .output()
                .unwrap()
                .stdout,
        )
        .unwrap()
    }

    #[tokio::test]
    async fn initializes_out_of_band_and_checkpoints_a_tool_write() {
        let dir = TempDir::new().unwrap();
        let workspace = dir.path().join("workspace");
        let checkpointer = WorkspaceCheckpointer::initialize(&workspace).unwrap();
        let mut tools = CheckpointingTool::wrap_all(
            vec![Box::new(WriteTool(workspace.join("answer.txt")))],
            checkpointer,
        );

        let result = tools
            .remove(0)
            .execute(json!({"body": "42"}))
            .await
            .unwrap();

        assert_eq!(result.output(), "written");
        assert!(workspace.join(".git").is_file());
        assert!(dir.path().join("workspace.git/HEAD").is_file());
        let history = log(&workspace);
        assert!(
            history.contains("checkpoint: after write_fixture"),
            "{history}"
        );
        assert!(
            history.contains("checkpoint: initialize workspace"),
            "{history}"
        );
    }

    #[tokio::test]
    async fn an_unchanged_tool_call_creates_no_checkpoint() {
        let dir = TempDir::new().unwrap();
        let workspace = dir.path().join("workspace");
        let checkpointer = WorkspaceCheckpointer::initialize(&workspace).unwrap();
        checkpointer.checkpoint("read_only").await.unwrap();
        assert_eq!(log(&workspace).lines().count(), 1);
    }

    #[tokio::test]
    async fn a_failed_checkpoint_preserves_the_successful_tool_result() {
        let dir = TempDir::new().unwrap();
        let workspace = dir.path().join("workspace");
        let checkpointer = WorkspaceCheckpointer::initialize(&workspace).unwrap();
        std::fs::remove_file(checkpointer.git_dir.join("HEAD")).unwrap();
        let mut tools = CheckpointingTool::wrap_all(
            vec![Box::new(WriteTool(workspace.join("answer.txt")))],
            checkpointer,
        );

        let result = tools
            .remove(0)
            .execute(json!({"body": "still written"}))
            .await
            .unwrap();

        assert_eq!(result.output(), "written");
        assert_eq!(
            std::fs::read_to_string(workspace.join("answer.txt")).unwrap(),
            "still written"
        );
    }

    #[tokio::test]
    async fn concurrent_tool_calls_serialize_the_git_index() {
        let dir = TempDir::new().unwrap();
        let workspace = dir.path().join("workspace");
        let checkpointer = WorkspaceCheckpointer::initialize(&workspace).unwrap();
        let mut tools = CheckpointingTool::wrap_all(
            vec![
                Box::new(WriteTool(workspace.join("one.txt"))),
                Box::new(WriteTool(workspace.join("two.txt"))),
            ],
            checkpointer,
        );
        let one = tools.remove(0);
        let two = tools.remove(0);

        let (one_result, two_result) = tokio::join!(
            one.execute(json!({"body": "one"})),
            two.execute(json!({"body": "two"}))
        );

        assert!(!one_result.unwrap().is_error);
        assert!(!two_result.unwrap().is_error);
        let status = Command::new("git")
            .args(["status", "--porcelain"])
            .current_dir(&workspace)
            .output()
            .unwrap();
        assert!(status.status.success());
        assert!(String::from_utf8(status.stdout).unwrap().trim().is_empty());
        assert!(!dir.path().join("workspace.git/index.lock").exists());
    }
}
