//! FR-018 host test (T011): a spawned tool child must NOT inherit the provider key env var, even
//! though the harness process holds it. A non-stripped var is the control — it *is* visible, so the
//! test proves the strip did the work (not that the env was empty).

use bee_harness::provider::mock_model::MockModel;
use bee_harness::provider::{ToolCall, Turn};
use bee_harness::sandbox::{self, Sandbox};
use bee_harness::scenario::Scenario;
use bee_harness::tools::registry_for;
use bee_harness::{run_loop, LoopOptions};

fn scenario() -> Scenario {
    Scenario {
        id: "cred".into(),
        policy_path: "/dev/null".into(),
        system_prompt: "sys".into(),
        task: "task".into(),
        turn_limit: 3,
        timeout_secs: 30,
        tools: vec!["bash".into()],
        mode: Default::default(),
        workdir: Default::default(),
        mcp: Default::default(),
    }
}

#[tokio::test]
async fn tool_child_cannot_read_provider_key() {
    // SAFETY: single-threaded within this test's control; set both a "secret" key var and a
    // non-secret control var in the harness (parent) environment.
    std::env::set_var("FAKE_PROVIDER_KEY", "sk-super-secret-value");
    std::env::set_var("FAKE_VISIBLE_VAR", "i-am-visible");

    let model = MockModel::scripted(vec![
        Turn::calls(vec![ToolCall {
            id: "1".into(),
            name: "bash".into(),
            arguments: serde_json::json!({
                "command": "echo KEY=[$FAKE_PROVIDER_KEY] VIS=[$FAKE_VISIBLE_VAR]"
            }),
        }]),
        Turn::text("done"),
    ]);

    let scn = scenario();
    let mut registry = registry_for(&scn.tools, None);
    // Strip list = default provider vars + the configured key var name.
    let mut sb = Sandbox::host(sandbox::key_vars(Some("FAKE_PROVIDER_KEY")));

    let t = run_loop(
        &model,
        &scn,
        &mut registry,
        &mut sb,
        &LoopOptions::default(),
    )
    .await;

    let out = &t.turns[0].calls[0].result.content;
    assert!(
        !out.contains("sk-super-secret-value"),
        "provider key leaked to tool child: {out}"
    );
    assert!(
        out.contains("KEY=[]"),
        "key var should be empty in the child: {out}"
    );
    // Control: a var we did NOT strip is still visible, proving the strip is selective.
    assert!(
        out.contains("i-am-visible"),
        "non-stripped control var missing: {out}"
    );

    std::env::remove_var("FAKE_PROVIDER_KEY");
    std::env::remove_var("FAKE_VISIBLE_VAR");
}
