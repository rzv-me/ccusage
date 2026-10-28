use std::process::{Command, Output};

use ccusage_test_support::{Fixture, fs_fixture};
use serde_json::{Value, json};

const SESSION_UUID: &str = "01a0bb8c-c7c0-7630-9d82-860b875930f0";
const SESSION_ID: &str =
    "2026/09/19/rollout-2026-09-19T23-22-40-01a0bb8c-c7c0-7630-9d82-860b875930f0";
const SESSION_FILE: &str = "rollout-2026-09-19T23-22-40-01a0bb8c-c7c0-7630-9d82-860b875930f0";
const OTHER_UUID: &str = "01a0cc9d-d8d1-8741-ae93-971c986041a1";

#[test]
fn selects_codex_session_by_supported_id_forms() {
    let fixture = codex_fixture();

    for requested_id in [
        SESSION_ID.to_string(),
        SESSION_FILE.to_string(),
        format!("{SESSION_FILE}.jsonl"),
        SESSION_UUID.to_string(),
        format!("codex://threads/{SESSION_UUID}"),
    ] {
        let output = run_cli(
            &fixture,
            &["codex", "session", "--id", &requested_id, "--json"],
        );
        let report: Value = serde_json::from_str(&output).unwrap();

        assert_eq!(report["sessionId"], SESSION_ID);
        assert_eq!(report["totalTokens"], 150);
        assert!(report.get("totalCost").is_some());
        assert!(report.get("costUSD").is_none());
        assert!(report.get("sessions").is_none());
        assert!(report.get("totals").is_none());
    }
}

#[test]
fn filters_codex_session_table_by_full_id() {
    let fixture = codex_fixture();

    let output = run_cli(
        &fixture,
        &["codex", "session", "--id", SESSION_ID, "--no-cost"],
    );

    assert!(output.contains(SESSION_UUID));
    assert!(!output.contains(OTHER_UUID));
    let selected_row = output
        .lines()
        .find(|line| line.contains(SESSION_UUID))
        .expect("selected session row should render");
    assert!(selected_row.contains("150"));
}

#[test]
fn reports_unknown_codex_session_id() {
    let fixture = codex_fixture();

    let output = run_cli_output(&fixture, &["codex", "session", "--id", "missing"]);

    assert!(!output.status.success());
    assert!(
        String::from_utf8(output.stderr)
            .unwrap()
            .contains("No Codex session found with ID: missing")
    );
}

#[test]
fn reports_ambiguous_codex_session_uuid() {
    let fixture = fs_fixture!({
        "codex/sessions/2026/09/19/rollout-2026-09-19T23-22-40-01a0bb8c-c7c0-7630-9d82-860b875930f0.jsonl": usage("2026-09-19T23:22:40.000Z", 100, 50),
        "codex/sessions/2026/09/20/rollout-2026-09-20T10-00-00-01a0bb8c-c7c0-7630-9d82-860b875930f0.jsonl": usage("2026-09-20T10:00:00.000Z", 200, 75),
    });

    let output = run_cli_output(&fixture, &["codex", "session", "--id", SESSION_UUID]);

    assert!(!output.status.success());
    assert!(String::from_utf8(output.stderr).unwrap().contains(&format!(
        "Codex session ID '{SESSION_UUID}' is ambiguous and matches 2 sessions."
    )));
}

#[test]
fn reports_compressed_codex_rollouts_alongside_plain_ones() {
    let fixture = codex_fixture();
    let compressed = fixture.write_file(
        format!("codex/archived_sessions/2026/09/21/{SESSION_FILE}-zst.jsonl.zst"),
        "",
    );
    std::fs::write(
        &compressed,
        zstd::stream::encode_all(usage("2026-09-21T08:00:00.000Z", 300, 25).as_bytes(), 3).unwrap(),
    )
    .unwrap();

    let daily: Value =
        serde_json::from_str(&run_cli(&fixture, &["codex", "daily", "--json"])).unwrap();
    let session: Value = serde_json::from_str(&run_cli(
        &fixture,
        &[
            "codex",
            "session",
            "--id",
            &format!("{SESSION_FILE}-zst.jsonl.zst"),
            "--json",
        ],
    ))
    .unwrap();

    assert_eq!(daily["daily"].as_array().unwrap().len(), 3);
    assert_eq!(daily["totals"]["totalTokens"], 150 + 1000 + 325);
    assert_eq!(
        session["sessionId"],
        format!("2026/09/21/{SESSION_FILE}-zst")
    );
    assert_eq!(session["totalTokens"], 325);
}

#[test]
fn reports_issue_1970_subagent_usage_in_daily_and_session_commands() {
    let fixture = issue_1970_codex_fixture();

    let daily_json = run_cli(
        &fixture,
        &[
            "codex", "daily", "--since", "20260101", "--until", "20260131", "--json",
        ],
    );
    let daily: Value = serde_json::from_str(&daily_json).unwrap();
    assert_eq!(daily["totals"]["totalTokens"], 1010);
    let daily_row = daily["daily"]
        .as_array()
        .unwrap()
        .iter()
        .find(|row| row["date"] == "2026-01-05")
        .expect("issue 1970 daily row should exist");
    assert_eq!(daily_row["totalTokens"], 1010);

    let session_json = run_cli(
        &fixture,
        &[
            "codex", "session", "--since", "20260101", "--until", "20260131", "--json",
        ],
    );
    let session: Value = serde_json::from_str(&session_json).unwrap();
    assert_eq!(session["totals"]["totalTokens"], 1010);
    let session_row = session["sessions"]
        .as_array()
        .unwrap()
        .iter()
        .find(|row| row["sessionFile"] == "child")
        .expect("issue 1970 session row should exist");
    assert_eq!(session_row["totalTokens"], 1010);

    insta::assert_snapshot!(
        "issue_1970_daily_table",
        format!(
            "Daily\n{}",
            run_cli(
                &fixture,
                &[
                    "codex", "daily", "--since", "20260101", "--until", "20260131",
                ],
            ),
        )
    );
    insta::assert_snapshot!(
        "issue_1970_session_table",
        format!(
            "Session\n{}",
            run_cli(
                &fixture,
                &[
                    "codex", "session", "--since", "20260101", "--until", "20260131",
                ],
            ),
        )
    );
}

#[test]
fn prices_ultrafast_in_focused_and_unified_reports() {
    let fixture = ultrafast_fixture();
    for kind in ["daily", "monthly", "session"] {
        let report: Value =
            serde_json::from_str(&run_cli(&fixture, &["codex", kind, "--json"])).unwrap();
        assert_eq!(report["totals"]["totalTokens"], 3_350);
        assert!((report["totals"]["costUSD"].as_f64().unwrap() - 0.02012).abs() < 1e-12);
    }
    let unified: Value = serde_json::from_str(&run_cli(&fixture, &["daily", "--json"])).unwrap();
    assert_eq!(unified["totals"]["totalTokens"], 3_350);
    assert!((unified["totals"]["totalCost"].as_f64().unwrap() - 0.02012).abs() < 1e-12);

    for (speed, expected) in [
        ("standard", 0.00752),
        ("fast", 0.01504),
        ("ultrafast", 0.04512),
    ] {
        let report: Value = serde_json::from_str(&run_cli(
            &fixture,
            &["codex", "daily", "--json", "--speed", speed],
        ))
        .unwrap();
        assert_eq!(report["totals"]["totalTokens"], 3_350);
        assert!((report["totals"]["costUSD"].as_f64().unwrap() - expected).abs() < 1e-12);
    }
}

#[test]
fn applies_ultrafast_speed_and_multiplier_from_ccusage_config() {
    let fixture = ultrafast_fixture();
    let config = fixture.write_file(
        "ccusage.json",
        json!({
            "codex": { "defaults": { "speed": "ultrafast" } },
            "defaults": { "pricingOverrides": { "gpt-6.1-sol": { "ultrafastMultiplier": 3 } } },
        })
        .to_string(),
    );
    let report: Value = serde_json::from_str(&run_cli(
        &fixture,
        &[
            "codex",
            "daily",
            "--json",
            "--config",
            config.to_str().unwrap(),
        ],
    ))
    .unwrap();
    assert!((report["totals"]["costUSD"].as_f64().unwrap() - 0.02256).abs() < 1e-12);
}

fn ultrafast_fixture() -> Fixture {
    let marker = |timestamp: &str, tier: &str| {
        json!({
        "timestamp": timestamp,
        "type": "event_msg",
        "payload": { "type": "thread_settings_applied", "thread_settings": { "service_tier": tier } },
    }).to_string()
    };
    let tokens = |timestamp: &str, input: u64, cache: u64, output: u64| {
        json!({
            "timestamp": timestamp,
            "type": "event_msg",
            "payload": { "type": "token_count", "info": {
                "model": "gpt-6.1-sol",
                "last_token_usage": { "input_tokens": input, "cached_input_tokens": cache,
                    "output_tokens": output, "total_tokens": input + output },
            } },
        })
        .to_string()
    };
    fs_fixture!({
        "codex/sessions/2026/10/27/ultrafast.jsonl": [
            marker("2026-10-27T10:00:00Z", "ultrafast"),
            tokens("2026-10-27T10:00:01Z", 1_200, 200, 50),
            marker("2026-10-27T10:01:00Z", "standard"),
            tokens("2026-10-27T10:01:01Z", 2_000, 0, 100),
        ].join("\n"),
    })
}

fn issue_1970_codex_fixture() -> Fixture {
    fs_fixture!({
        "codex/sessions/2026/01/05/child.jsonl":
            include_str!("fixtures/codex/issue_1970.jsonl"),
    })
}

fn codex_fixture() -> Fixture {
    fs_fixture!({
        "codex/sessions/2026/09/19/rollout-2026-09-19T23-22-40-01a0bb8c-c7c0-7630-9d82-860b875930f0.jsonl": usage("2026-09-19T23:22:40.000Z", 100, 50),
        "codex/sessions/2026/09/20/rollout-2026-09-20T10-00-00-01a0cc9d-d8d1-8741-ae93-971c986041a1.jsonl": usage("2026-09-20T10:00:00.000Z", 900, 100),
    })
}

fn usage(timestamp: &str, input_tokens: u64, output_tokens: u64) -> String {
    json!({
        "timestamp": timestamp,
        "type": "event_msg",
        "payload": {
            "type": "token_count",
            "info": {
                "model": "gpt-5",
                "last_token_usage": {
                    "input_tokens": input_tokens,
                    "cached_input_tokens": 10,
                    "output_tokens": output_tokens,
                    "reasoning_output_tokens": 0,
                    "total_tokens": input_tokens + output_tokens,
                },
            },
        },
    })
    .to_string()
}

fn run_cli(fixture: &Fixture, args: &[&str]) -> String {
    let output = run_cli_output(fixture, args);
    assert!(
        output.status.success(),
        "ccusage CLI failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    String::from_utf8(output.stdout).expect("ccusage CLI stdout should be UTF-8")
}

fn run_cli_output(fixture: &Fixture, args: &[&str]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_ccusage"))
        .args(args)
        .args(["--offline", "--no-color", "--timezone", "UTC"])
        .env("CODEX_HOME", fixture.path("codex"))
        .env("HOME", fixture.path("empty-home"))
        .env("USERPROFILE", fixture.path("empty-userprofile"))
        .env("XDG_CONFIG_HOME", fixture.path("empty-xdg-config"))
        .env("LOG_LEVEL", "0")
        .env("NO_COLOR", "1")
        .env("COLUMNS", "240")
        .env_remove("CLAUDE_CONFIG_DIR")
        .output()
        .expect("ccusage CLI should run")
}
