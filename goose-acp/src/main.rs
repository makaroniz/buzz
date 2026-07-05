//! Slim goose agent as an ACP stdio server.
//!
//! Buzz talks to agents over ACP (see `buzz-acp`); this binary lets the Buzz
//! distribution bundle a capable goose agent without requiring users to
//! install goose. It is the entire integration: goose's public ACP server on
//! stdin/stdout, with no builtin-extension overrides (goose registers its
//! defaults, e.g. developer, itself).

fn main() -> anyhow::Result<()> {
    tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()?
        .block_on(goose::acp::server::run(Vec::new()))
}
