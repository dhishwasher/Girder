//! The parallel agent-swarm orchestrator.
//!
//! Spawns every agent as an independent tokio task subscribed to the bus, then
//! injects an intent and lets the swarm collaborate. CPU-bound graph work the
//! agents do is cheap here, but the design routes heavy analysis through
//! `rayon` (see [`Agent`] notes). The shared graph is the single source of
//! truth all agents read and mutate.

use crate::agents::{Agent, AgentResult};
use crate::bus::{self, Bus, MsgKind, Role, SwarmMessage};
use aether_ai::Router;
use aether_graph::SemanticGraph;
use std::sync::{Arc, Mutex};
use std::time::Duration;

/// Shared context every agent receives: the AI router and the live graph.
pub struct SwarmContext {
    pub router: Router,
    pub graph: Arc<Mutex<SemanticGraph>>,
    /// The module new code is attached to, e.g. `crate::math`.
    pub target_module: String,
    /// The file projection for that module.
    pub target_file: String,
}

impl SwarmContext {
    pub fn new(router: Router, graph: Arc<Mutex<SemanticGraph>>, module: &str, file: &str) -> Self {
        SwarmContext {
            router,
            graph,
            target_module: module.to_string(),
            target_file: file.to_string(),
        }
    }
}

/// Runs the swarm and returns a transcript of everything that was said.
pub struct Orchestrator {
    ctx: Arc<SwarmContext>,
    agents: Vec<Arc<dyn Agent>>,
}

impl Orchestrator {
    pub fn new(ctx: Arc<SwarmContext>) -> Self {
        Orchestrator {
            ctx,
            agents: Vec::new(),
        }
    }

    /// Register the full default swarm (3 live agents + 4 EXTENSION-POINT stubs
    /// + the QueryAgent for knowledge-graph questions).
    pub fn with_default_swarm(mut self) -> Self {
        use crate::agents::*;
        self.agents = vec![
            Arc::new(PlannerAgent),
            Arc::new(CoderAgent),
            Arc::new(TesterAgent),
            Arc::new(DocumenterAgent),
            Arc::new(RefactorerAgent),
            Arc::new(OptimizerAgent),
            Arc::new(SecurityAuditorAgent),
            Arc::new(QueryAgent),
        ];
        self
    }

    pub fn add_agent(&mut self, agent: Arc<dyn Agent>) {
        self.agents.push(agent);
    }

    /// Run only the [`PlannerAgent`] for `intent` and return its messages.
    ///
    /// Used by the `girder plan` CLI command to preview what the swarm
    /// *would* build — graph context is gathered and fn specs are produced,
    /// but the Coder never runs and no code is written to the graph.
    pub async fn plan_only(&self, intent: &str) -> Vec<SwarmMessage> {
        use crate::agents::AgentResult;
        let trigger = SwarmMessage::new(Role::Conductor, MsgKind::Intent(intent.to_string()));
        for agent in &self.agents {
            if agent.role() == Role::Planner {
                if let AgentResult::Emit(msgs) = agent.handle(&trigger, &self.ctx).await {
                    return msgs;
                }
                break;
            }
        }
        vec![]
    }

    /// Run only the [`QueryAgent`] for `question` and return the answer as a
    /// `Note` message (or an empty vec if the question cannot be parsed).
    ///
    /// Used by `girder query` to answer knowledge-graph questions without
    /// touching the graph or running any code generation.
    pub async fn query_only(&self, question: &str) -> Vec<SwarmMessage> {
        use crate::agents::AgentResult;
        let trigger = SwarmMessage::new(Role::Conductor, MsgKind::Intent(question.to_string()));
        for agent in &self.agents {
            if agent.role() == Role::QueryAgent {
                if let AgentResult::Emit(msgs) = agent.handle(&trigger, &self.ctx).await {
                    return msgs;
                }
                break;
            }
        }
        vec![]
    }

    /// Inject `intent`, run the swarm concurrently, and collect the transcript.
    ///
    /// Terminates on **quiescence** — when no new message has arrived for a short
    /// grace window — or after `budget` elapses, whichever comes first. Because
    /// every agent emits a message after it finishes mutating the graph,
    /// quiescence guarantees all in-flight graph mutations are complete before
    /// the caller reads the graph (no teardown race).
    pub async fn run(&self, intent: &str, budget: Duration) -> Vec<SwarmMessage> {
        let bus: Bus = bus::channel(256);

        // Subscribe + spawn every agent BEFORE any message is sent so none miss
        // the opening intent.
        let mut handles = Vec::new();
        for agent in &self.agents {
            let mut rx = bus.subscribe();
            let tx = bus.clone();
            let ctx = self.ctx.clone();
            let agent = agent.clone();
            handles.push(tokio::spawn(async move {
                while let Ok(msg) = rx.recv().await {
                    // An agent never reacts to its own broadcast.
                    if msg.from == agent.role() {
                        continue;
                    }
                    if let AgentResult::Emit(out) = agent.handle(&msg, &ctx).await {
                        for m in out {
                            let _ = tx.send(m);
                        }
                    }
                }
            }));
        }

        // The conductor's own view of the conversation. Subscribed before the
        // intent is sent, so it captures the whole exchange including the kickoff.
        let mut transcript_rx = bus.subscribe();
        let intent_msg = SwarmMessage::new(Role::Conductor, MsgKind::Intent(intent.to_string()));
        let _ = bus.send(intent_msg);

        // Grace window: once the bus has been silent this long, the swarm has
        // settled. Small enough to feel instant, large enough to drain handlers.
        let quiescence = Duration::from_millis(150);
        let deadline = std::time::Instant::now() + budget;
        let mut transcript = Vec::new();
        loop {
            let remaining = deadline.saturating_duration_since(std::time::Instant::now());
            if remaining.is_zero() {
                break;
            }
            let wait = quiescence.min(remaining);
            match tokio::time::timeout(wait, transcript_rx.recv()).await {
                Ok(Ok(msg)) => transcript.push(msg),
                Ok(Err(_)) => break, // bus closed
                Err(_) => break,     // quiescence reached
            }
        }

        // Tear down: dropping the bus closes every agent's receiver loop.
        drop(bus);
        for h in handles {
            h.abort();
        }
        transcript
    }
}
