use carve_agent::{
    execute_single_tool, AgentPlugin, FsSkillRegistry, LoadSkillReferenceTool, Message, Phase,
    Session, SkillActivateTool, SkillRegistry, SkillRuntimePlugin, SkillScriptTool, StepContext,
    ToolCall, ToolDescriptor, ToolResult,
};
use serde_json::json;
use std::fs;
use std::io::Write;
use std::sync::Arc;
use tempfile::TempDir;

fn make_skill_tree() -> (TempDir, Arc<dyn SkillRegistry>) {
    let td = TempDir::new().unwrap();
    let skills_root = td.path().join("skills");
    let docx_root = skills_root.join("docx");
    fs::create_dir_all(docx_root.join("references")).unwrap();
    fs::create_dir_all(docx_root.join("scripts")).unwrap();

    fs::write(
        docx_root.join("references").join("DOCX-JS.md"),
        "Use docx-js for new documents.",
    )
    .unwrap();

    let mut f = fs::File::create(docx_root.join("SKILL.md")).unwrap();
    writeln!(
        f,
        "{}",
        r#"---
name: docx
description: DOCX processing guidance
allowed-tools: read_file
---
# DOCX Processing

## Creating documents

Use docx-js for new documents. See [DOCX-JS.md](references/DOCX-JS.md).
"#
    )
    .unwrap();

    fs::write(
        docx_root.join("scripts").join("hello.sh"),
        r#"#!/usr/bin/env bash
echo "hello"
"#,
    )
    .unwrap();

    let reg: Arc<dyn SkillRegistry> =
        Arc::new(FsSkillRegistry::discover_root(skills_root).unwrap());
    (td, reg)
}

async fn apply_tool(
    session: Session,
    tool: &dyn carve_agent::Tool,
    call: ToolCall,
) -> (Session, ToolResult) {
    let state = session.rebuild_state().unwrap();
    let exec = execute_single_tool(Some(tool), &call, &state).await;
    let session = if let Some(patch) = exec.patch.clone() {
        session.with_patch(patch)
    } else {
        session
    };
    (session, exec.result)
}

#[tokio::test]
async fn test_skill_activation_injects_instructions() {
    let (_td, reg) = make_skill_tree();
    let activate = SkillActivateTool::new(reg);
    let plugin = SkillRuntimePlugin::new();

    let session = Session::with_initial_state("s", json!({})).with_message(Message::user("hi"));

    let (session, result) = apply_tool(
        session,
        &activate,
        ToolCall::new("call_1", "skill", json!({"skill": "docx"})),
    )
    .await;
    assert!(result.is_success());

    let mut step = StepContext::new(&session, vec![ToolDescriptor::new("x", "x", "x")]);
    plugin.on_phase(Phase::BeforeInference, &mut step).await;

    assert_eq!(step.system_context.len(), 1);
    let injected = &step.system_context[0];
    assert!(injected.contains("<skill id=\"docx\">"));
    assert!(injected.contains("DOCX Processing"));
    assert!(!injected.contains("<skill_reference"));
    assert!(!injected.contains("<skill_script_result"));
}

#[tokio::test]
async fn test_load_reference_injects_reference_content() {
    let (_td, reg) = make_skill_tree();
    let activate = SkillActivateTool::new(reg.clone());
    let load_ref = LoadSkillReferenceTool::new(reg);
    let plugin = SkillRuntimePlugin::new();

    let session = Session::with_initial_state("s", json!({})).with_message(Message::user("hi"));

    let (session, _) = apply_tool(
        session,
        &activate,
        ToolCall::new("call_1", "skill", json!({"skill": "docx"})),
    )
    .await;

    let (session, result) = apply_tool(
        session,
        &load_ref,
        ToolCall::new(
            "call_2",
            "load_skill_reference",
            json!({"skill": "docx", "path": "references/DOCX-JS.md"}),
        ),
    )
    .await;
    assert!(result.is_success());

    let mut step = StepContext::new(&session, vec![ToolDescriptor::new("x", "x", "x")]);
    plugin.on_phase(Phase::BeforeInference, &mut step).await;
    let injected = &step.system_context[0];
    assert!(injected.contains("<skill_reference"));
    assert!(injected.contains("Use docx-js for new documents."));
}

#[tokio::test]
async fn test_script_result_is_persisted_and_injected() {
    let (_td, reg) = make_skill_tree();
    let activate = SkillActivateTool::new(reg.clone());
    let run_script = SkillScriptTool::new(reg);
    let plugin = SkillRuntimePlugin::new();

    let session = Session::with_initial_state("s", json!({})).with_message(Message::user("hi"));

    let (session, _) = apply_tool(
        session,
        &activate,
        ToolCall::new("call_1", "skill", json!({"skill": "docx"})),
    )
    .await;

    let (session, result) = apply_tool(
        session,
        &run_script,
        ToolCall::new(
            "call_2",
            "skill_script",
            json!({"skill": "docx", "script": "scripts/hello.sh"}),
        ),
    )
    .await;
    assert!(result.is_success());

    let mut step = StepContext::new(&session, vec![ToolDescriptor::new("x", "x", "x")]);
    plugin.on_phase(Phase::BeforeInference, &mut step).await;
    let injected = &step.system_context[0];
    assert!(injected.contains("<skill_script_result"));
    assert!(injected.contains("hello"));
}

#[tokio::test]
async fn test_load_reference_rejects_escape() {
    let (_td, reg) = make_skill_tree();
    let load_ref = LoadSkillReferenceTool::new(reg);

    let session = Session::with_initial_state("s", json!({}));
    let (_session, result) = apply_tool(
        session,
        &load_ref,
        ToolCall::new(
            "call_1",
            "load_skill_reference",
            json!({"skill": "docx", "path": "../secrets.txt"}),
        ),
    )
    .await;

    assert!(result.is_error());
    let msg = result.message.clone().unwrap_or_default();
    assert!(
        msg.contains("Invalid arguments") || msg.to_lowercase().contains("invalid"),
        "expected invalid path error"
    );
}

#[tokio::test]
async fn test_skill_activation_requires_exact_skill_name() {
    let (_td, reg) = make_skill_tree();
    let activate = SkillActivateTool::new(reg);

    let session = Session::with_initial_state("s", json!({}));
    let (_session, result) = apply_tool(
        session,
        &activate,
        ToolCall::new("call_1", "skill", json!({"skill": "DOCX"})),
    )
    .await;

    assert!(result.is_error());
}

#[tokio::test]
async fn test_skill_activation_unknown_skill_errors() {
    let (_td, reg) = make_skill_tree();
    let activate = SkillActivateTool::new(reg);

    let session = Session::with_initial_state("s", json!({}));
    let (_session, result) = apply_tool(
        session,
        &activate,
        ToolCall::new("call_1", "skill", json!({"skill": "nope"})),
    )
    .await;

    assert!(result.is_error());
}

#[tokio::test]
async fn test_skill_activation_missing_skill_argument_is_error() {
    let (_td, reg) = make_skill_tree();
    let activate = SkillActivateTool::new(reg);

    let session = Session::with_initial_state("s", json!({}));
    let (_session, result) = apply_tool(
        session,
        &activate,
        ToolCall::new("call_1", "skill", json!({})),
    )
    .await;

    assert!(result.is_error());
}

#[tokio::test]
async fn test_skill_activation_applies_allowed_tools_to_permission_state() {
    let (_td, reg) = make_skill_tree();
    let activate = SkillActivateTool::new(reg);

    let session = Session::with_initial_state("s", json!({}));
    let (session, result) = apply_tool(
        session,
        &activate,
        ToolCall::new("call_1", "skill", json!({"skill": "docx"})),
    )
    .await;
    assert!(result.is_success());

    let state = session.rebuild_state().unwrap();
    assert_eq!(state["permissions"]["tools"]["read_file"], "allow");
}

#[tokio::test]
async fn test_skill_activation_skill_md_removed_after_discovery_still_works() {
    let (td, reg) = make_skill_tree();
    // Ensure discovery has produced the meta and cached SKILL.md content.
    assert_eq!(reg.list().len(), 1);
    fs::remove_file(td.path().join("skills").join("docx").join("SKILL.md")).unwrap();

    let activate = SkillActivateTool::new(reg);
    let session = Session::with_initial_state("s", json!({}));
    let (_session, result) = apply_tool(
        session,
        &activate,
        ToolCall::new("call_1", "skill", json!({"skill": "docx"})),
    )
    .await;

    assert!(result.is_success());
}

#[tokio::test]
async fn test_load_reference_requires_references_prefix() {
    let (_td, reg) = make_skill_tree();
    let load_ref = LoadSkillReferenceTool::new(reg);

    let session = Session::with_initial_state("s", json!({}));
    let (_session, result) = apply_tool(
        session,
        &load_ref,
        ToolCall::new(
            "call_1",
            "load_skill_reference",
            json!({"skill": "docx", "path": "SKILL.md"}),
        ),
    )
    .await;

    assert!(result.is_error());
}

#[tokio::test]
async fn test_load_reference_missing_arguments_are_errors() {
    let (_td, reg) = make_skill_tree();
    let load_ref = LoadSkillReferenceTool::new(reg);

    let session = Session::with_initial_state("s", json!({}));

    let (_session, r1) = apply_tool(
        session.clone(),
        &load_ref,
        ToolCall::new("call_1", "load_skill_reference", json!({"skill": "docx"})),
    )
    .await;
    assert!(r1.is_error());

    let (_session, r2) = apply_tool(
        session,
        &load_ref,
        ToolCall::new(
            "call_2",
            "load_skill_reference",
            json!({"path": "references/DOCX-JS.md"}),
        ),
    )
    .await;
    assert!(r2.is_error());
}

#[tokio::test]
async fn test_load_reference_invalid_utf8_is_error() {
    let (td, reg) = make_skill_tree();
    let load_ref = LoadSkillReferenceTool::new(reg);

    let refs_dir = td.path().join("skills").join("docx").join("references");
    fs::write(refs_dir.join("BAD.bin"), vec![0xff, 0xfe, 0xfd]).unwrap();

    let session = Session::with_initial_state("s", json!({}));
    let (_session, result) = apply_tool(
        session,
        &load_ref,
        ToolCall::new(
            "call_1",
            "load_skill_reference",
            json!({"skill": "docx", "path": "references/BAD.bin"}),
        ),
    )
    .await;

    assert!(result.is_error());
}

#[tokio::test]
async fn test_load_reference_missing_file_is_error() {
    let (_td, reg) = make_skill_tree();
    let load_ref = LoadSkillReferenceTool::new(reg);

    let session = Session::with_initial_state("s", json!({}));
    let (_session, result) = apply_tool(
        session,
        &load_ref,
        ToolCall::new(
            "call_1",
            "load_skill_reference",
            json!({"skill": "docx", "path": "references/DOES_NOT_EXIST.md"}),
        ),
    )
    .await;

    assert!(result.is_error());
}

#[cfg(unix)]
#[tokio::test]
async fn test_load_reference_symlink_escape_is_error() {
    use std::os::unix::fs as unix_fs;

    let (td, reg) = make_skill_tree();
    let load_ref = LoadSkillReferenceTool::new(reg);

    let outside = td.path().join("outside.md");
    fs::write(&outside, "outside").unwrap();

    let refs_dir = td.path().join("skills").join("docx").join("references");
    unix_fs::symlink(&outside, refs_dir.join("ESCAPE.md")).unwrap();

    let session = Session::with_initial_state("s", json!({}));
    let (_session, result) = apply_tool(
        session,
        &load_ref,
        ToolCall::new(
            "call_1",
            "load_skill_reference",
            json!({"skill": "docx", "path": "references/ESCAPE.md"}),
        ),
    )
    .await;

    assert!(result.is_error());
    let msg = result.message.clone().unwrap_or_default().to_lowercase();
    assert!(
        msg.contains("escapes")
            || msg.contains("invalid")
            || msg.contains("outside skill root")
            || msg.contains("outside")
    );
}

#[tokio::test]
async fn test_script_requires_scripts_prefix() {
    let (_td, reg) = make_skill_tree();
    let run_script = SkillScriptTool::new(reg);

    let session = Session::with_initial_state("s", json!({}));
    let (_session, result) = apply_tool(
        session,
        &run_script,
        ToolCall::new(
            "call_1",
            "skill_script",
            json!({"skill": "docx", "script": "references/DOCX-JS.md"}),
        ),
    )
    .await;

    assert!(result.is_error());
}

#[tokio::test]
async fn test_script_missing_arguments_are_errors() {
    let (_td, reg) = make_skill_tree();
    let run_script = SkillScriptTool::new(reg);

    let session = Session::with_initial_state("s", json!({}));

    let (_session, r1) = apply_tool(
        session.clone(),
        &run_script,
        ToolCall::new("call_1", "skill_script", json!({"skill": "docx"})),
    )
    .await;
    assert!(r1.is_error());

    let (_session, r2) = apply_tool(
        session,
        &run_script,
        ToolCall::new(
            "call_2",
            "skill_script",
            json!({"script": "scripts/hello.sh"}),
        ),
    )
    .await;
    assert!(r2.is_error());
}

#[tokio::test]
async fn test_script_args_are_passed_through() {
    let (td, reg) = make_skill_tree();
    let activate = SkillActivateTool::new(reg.clone());
    let run_script = SkillScriptTool::new(reg);
    let plugin = SkillRuntimePlugin::new();

    let scripts_dir = td.path().join("skills").join("docx").join("scripts");
    fs::write(
        scripts_dir.join("echo_args.sh"),
        r#"#!/usr/bin/env bash
printf "%s" "$*"
"#,
    )
    .unwrap();

    let session = Session::with_initial_state("s", json!({}));
    let (session, _) = apply_tool(
        session,
        &activate,
        ToolCall::new("call_1", "skill", json!({"skill": "docx"})),
    )
    .await;

    let (session, result) = apply_tool(
        session,
        &run_script,
        ToolCall::new(
            "call_2",
            "skill_script",
            json!({"skill": "docx", "script": "scripts/echo_args.sh", "args": ["a", "b"]}),
        ),
    )
    .await;
    assert!(result.is_success());

    let mut step = StepContext::new(&session, vec![ToolDescriptor::new("x", "x", "x")]);
    plugin.on_phase(Phase::BeforeInference, &mut step).await;
    let injected = &step.system_context[0];
    assert!(injected.contains("<stdout>"));
    assert!(injected.contains("a b"));
}

#[tokio::test]
async fn test_script_nonzero_exit_sets_ok_false() {
    let (td, reg) = make_skill_tree();
    let run_script = SkillScriptTool::new(reg);

    let scripts_dir = td.path().join("skills").join("docx").join("scripts");
    fs::write(
        scripts_dir.join("fail.sh"),
        r#"#!/usr/bin/env bash
echo "nope"
exit 2
"#,
    )
    .unwrap();

    let session = Session::with_initial_state("s", json!({}));
    let (_session, result) = apply_tool(
        session,
        &run_script,
        ToolCall::new(
            "call_1",
            "skill_script",
            json!({"skill": "docx", "script": "scripts/fail.sh"}),
        ),
    )
    .await;

    assert!(result.is_success());
    assert_eq!(result.data["ok"], false);
    assert_eq!(result.data["exit_code"], 2);
}

#[tokio::test]
async fn test_script_unsupported_runtime_is_error() {
    let (td, reg) = make_skill_tree();
    // Create a script with an unsupported extension.
    let scripts_dir = td.path().join("skills").join("docx").join("scripts");
    fs::write(scripts_dir.join("bad.rb"), "puts 'hi'\n").unwrap();

    let run_script = SkillScriptTool::new(reg);
    let session = Session::with_initial_state("s", json!({}));
    let (_session, result) = apply_tool(
        session,
        &run_script,
        ToolCall::new(
            "call_1",
            "skill_script",
            json!({"skill": "docx", "script": "scripts/bad.rb"}),
        ),
    )
    .await;

    assert!(result.is_error());
}

#[tokio::test]
async fn test_reference_truncation_flag_is_injected() {
    let (td, reg) = make_skill_tree();
    let activate = SkillActivateTool::new(reg.clone());
    let load_ref = LoadSkillReferenceTool::new(reg);
    let plugin = SkillRuntimePlugin::new();

    // Create a big reference file (>256KiB).
    let big = "a".repeat(257 * 1024);
    let refs_dir = td.path().join("skills").join("docx").join("references");
    fs::write(refs_dir.join("BIG.md"), big).unwrap();

    let session = Session::with_initial_state("s", json!({}));
    let (session, _) = apply_tool(
        session,
        &activate,
        ToolCall::new("call_1", "skill", json!({"skill": "docx"})),
    )
    .await;

    let (session, result) = apply_tool(
        session,
        &load_ref,
        ToolCall::new(
            "call_2",
            "load_skill_reference",
            json!({"skill": "docx", "path": "references/BIG.md"}),
        ),
    )
    .await;
    assert!(result.is_success());

    let mut step = StepContext::new(&session, vec![ToolDescriptor::new("x", "x", "x")]);
    plugin.on_phase(Phase::BeforeInference, &mut step).await;
    let injected = &step.system_context[0];
    assert!(injected.contains("path=\"references/BIG.md\""));
    assert!(injected.contains("truncated=\"true\""));
}

#[tokio::test]
async fn test_script_stdout_truncation_flag_is_injected() {
    let (td, reg) = make_skill_tree();
    let activate = SkillActivateTool::new(reg.clone());
    let run_script = SkillScriptTool::new(reg);
    let plugin = SkillRuntimePlugin::new();

    // Print >32KiB to stdout.
    let scripts_dir = td.path().join("skills").join("docx").join("scripts");
    fs::write(
        scripts_dir.join("big.sh"),
        r#"#!/usr/bin/env bash
head -c 40000 /dev/zero | tr '\0' 'a'
"#,
    )
    .unwrap();

    let session = Session::with_initial_state("s", json!({}));
    let (session, _) = apply_tool(
        session,
        &activate,
        ToolCall::new("call_1", "skill", json!({"skill": "docx"})),
    )
    .await;

    let (session, result) = apply_tool(
        session,
        &run_script,
        ToolCall::new(
            "call_2",
            "skill_script",
            json!({"skill": "docx", "script": "scripts/big.sh"}),
        ),
    )
    .await;
    assert!(result.is_success());

    let mut step = StepContext::new(&session, vec![ToolDescriptor::new("x", "x", "x")]);
    plugin.on_phase(Phase::BeforeInference, &mut step).await;
    let injected = &step.system_context[0];
    assert!(injected.contains("script=\"scripts/big.sh\""));
    assert!(injected.contains("stdout_truncated=\"true\""));
}
