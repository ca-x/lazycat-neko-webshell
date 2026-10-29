#[path = "../agent_daemon.rs"]
mod agent_daemon;
#[path = "../agent_history.rs"]
mod agent_history;
#[path = "../agent_protocol.rs"]
#[expect(
    dead_code,
    reason = "The agent shares protocol encoders with the async provider"
)]
mod agent_protocol;
#[path = "../agent_pty.rs"]
mod agent_pty;
#[path = "../agent_status_hook.rs"]
mod agent_status_hook;
#[path = "../agent_workspace.rs"]
mod agent_workspace;
#[path = "../config.rs"]
#[expect(
    dead_code,
    reason = "Shared limits also include provider-only settings"
)]
mod config;
#[path = "../herdr_machines.rs"]
#[expect(
    dead_code,
    reason = "The provider owns machine authorization and compatibility checks"
)]
mod herdr_machines;
#[path = "../proto.rs"]
mod proto;
#[path = "../pty_io.rs"]
#[cfg_attr(
    not(test),
    expect(
        dead_code,
        reason = "Connection-scoped timeout waits are used only by the provider"
    )
)]
mod pty_io;
#[path = "../restty_headless.rs"]
mod restty_headless;
#[path = "../terminal_reply_authority.rs"]
mod terminal_reply_authority;
#[path = "../tty_init.rs"]
#[expect(
    dead_code,
    reason = "Feature gating applies only to provider entry points"
)]
mod tty_init;
#[path = "../validation.rs"]
#[expect(
    dead_code,
    reason = "RPC field normalization is used only by the provider"
)]
mod validation;

fn main() -> anyhow::Result<()> {
    let args = std::env::args().skip(1).collect::<Vec<_>>();
    let agent_args = if args.first().is_some_and(|argument| argument == "agent") {
        &args[1..]
    } else {
        args.as_slice()
    };
    agent_daemon::run_agent_command(agent_args)
}
