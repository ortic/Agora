//! Agora: an internal AI platform. Chat with Claude, OpenAI-compatible and local models, with
//! single sign-on, usage tracking and budgets, tracked web search and a tamper-evident audit log.

pub mod app;
pub mod audit;
pub mod auth;
pub mod catalog;
pub mod chat;
pub mod config;
pub mod connectors;
pub mod crypto;
pub mod desktop;
pub mod error;
pub mod llm;
pub mod maintenance;
pub mod mcp;
pub mod routes;
pub mod settings;
pub mod sso;
pub mod usage;
pub mod web;
pub mod web_assets;
