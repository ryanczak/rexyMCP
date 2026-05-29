use crate::ai::AiClient;
use crate::ai::types::{AiEvent, Message, ToolSchema};
use async_trait::async_trait;
use std::collections::VecDeque;
use std::sync::{Arc, Mutex};
use tokio::sync::mpsc::UnboundedSender;

#[derive(Debug, Clone)]
pub struct MockCall {
    pub system_prompt: String,
    pub messages: Vec<Message>,
    pub tool_count: usize,
}

#[derive(Clone)]
pub struct MockAiClient {
    script: Arc<Mutex<VecDeque<String>>>,
    calls: Arc<Mutex<Vec<MockCall>>>,
}

impl MockAiClient {
    pub fn new(script: Vec<String>) -> Self {
        Self {
            script: Arc::new(Mutex::new(script.into_iter().collect())),
            calls: Arc::new(Mutex::new(Vec::new())),
        }
    }

    pub fn calls(&self) -> Vec<MockCall> {
        self.calls.lock().unwrap().clone()
    }
}

#[async_trait]
impl AiClient for MockAiClient {
    async fn chat(
        &self,
        system_prompt: &str,
        messages: Vec<Message>,
        tx: UnboundedSender<AiEvent>,
        tools: Option<&[ToolSchema]>,
    ) -> anyhow::Result<()> {
        self.calls.lock().unwrap().push(MockCall {
            system_prompt: system_prompt.to_string(),
            messages,
            tool_count: tools.map(|t| t.len()).unwrap_or(0),
        });
        let next = self.script.lock().unwrap().pop_front();
        if let Some(s) = next {
            let _ = tx.send(AiEvent::Token(s));
        }
        Ok(())
    }
}

pub struct MockAiClientEvents {
    script: Arc<Mutex<VecDeque<AiEvent>>>,
    calls: Arc<Mutex<Vec<MockCall>>>,
}

impl MockAiClientEvents {
    pub fn new(script: Vec<AiEvent>) -> Self {
        Self {
            script: Arc::new(Mutex::new(script.into_iter().collect())),
            calls: Arc::new(Mutex::new(Vec::new())),
        }
    }

    pub fn calls(&self) -> Vec<MockCall> {
        self.calls.lock().unwrap().clone()
    }
}

#[async_trait]
impl AiClient for MockAiClientEvents {
    async fn chat(
        &self,
        system_prompt: &str,
        messages: Vec<Message>,
        tx: UnboundedSender<AiEvent>,
        tools: Option<&[ToolSchema]>,
    ) -> anyhow::Result<()> {
        self.calls.lock().unwrap().push(MockCall {
            system_prompt: system_prompt.to_string(),
            messages,
            tool_count: tools.map(|t| t.len()).unwrap_or(0),
        });
        while let Some(event) = self.script.lock().unwrap().pop_front() {
            let _ = tx.send(event);
        }
        Ok(())
    }
}

/// Event-scripted mock that yields one inner script **per `chat` call** — unlike
/// `MockAiClientEvents`, which drains its whole script in a single call. Each
/// `chat` pops the next `Vec<AiEvent>` and sends it; an exhausted script sends
/// nothing (an empty completion). Lets a test drive a multi-turn loop where each
/// turn emits its own native/text/done sequence.
#[derive(Clone)]
pub struct MockAiClientScript {
    turns: Arc<Mutex<VecDeque<Vec<AiEvent>>>>,
    calls: Arc<Mutex<Vec<MockCall>>>,
}

impl MockAiClientScript {
    pub fn new(turns: Vec<Vec<AiEvent>>) -> Self {
        Self {
            turns: Arc::new(Mutex::new(turns.into_iter().collect())),
            calls: Arc::new(Mutex::new(Vec::new())),
        }
    }

    pub fn calls(&self) -> Vec<MockCall> {
        self.calls.lock().unwrap().clone()
    }
}

#[async_trait]
impl AiClient for MockAiClientScript {
    async fn chat(
        &self,
        system_prompt: &str,
        messages: Vec<Message>,
        tx: UnboundedSender<AiEvent>,
        tools: Option<&[ToolSchema]>,
    ) -> anyhow::Result<()> {
        self.calls.lock().unwrap().push(MockCall {
            system_prompt: system_prompt.to_string(),
            messages,
            tool_count: tools.map(|t| t.len()).unwrap_or(0),
        });
        let next = self.turns.lock().unwrap().pop_front();
        if let Some(events) = next {
            for event in events {
                let _ = tx.send(event);
            }
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ai::TokenBreakdown;
    use tokio::sync::mpsc;

    #[tokio::test]
    async fn mock_ai_client_records_call_and_plays_token() {
        let mock = MockAiClient::new(vec!["hello from mock".to_string()]);
        let (tx, mut rx) = mpsc::unbounded_channel::<AiEvent>();

        mock.chat("system", vec![], tx, None).await.unwrap();

        let event = rx.try_recv().expect("expected a token event");
        match event {
            AiEvent::Token(t) => assert_eq!(t, "hello from mock"),
            other => panic!("expected Token, got {:?}", other),
        }

        let calls = mock.calls();
        assert_eq!(calls.len(), 1);
        assert_eq!(calls[0].system_prompt, "system");
        assert_eq!(calls[0].tool_count, 0);
    }

    #[tokio::test]
    async fn mock_ai_client_reports_tool_count() {
        let mock = MockAiClient::new(vec![]);
        let (tx, _rx) = mpsc::unbounded_channel::<AiEvent>();
        let tools = vec![
            ToolSchema {
                name: "a".into(),
                description: "a".into(),
                parameters: serde_json::json!({}),
            },
            ToolSchema {
                name: "b".into(),
                description: "b".into(),
                parameters: serde_json::json!({}),
            },
        ];

        mock.chat("sys", vec![], tx, Some(&tools)).await.unwrap();

        let calls = mock.calls();
        assert_eq!(calls[0].tool_count, 2);
    }

    #[tokio::test]
    async fn mock_ai_client_empty_tools_reports_zero() {
        let mock = MockAiClient::new(vec![]);
        let (tx, _rx) = mpsc::unbounded_channel::<AiEvent>();

        mock.chat("sys", vec![], tx, Some(&[])).await.unwrap();

        let calls = mock.calls();
        assert_eq!(calls[0].tool_count, 0);
    }

    #[tokio::test]
    async fn mock_ai_client_events_plays_structured_events() {
        let events = vec![
            AiEvent::Token("token1".to_string()),
            AiEvent::Token("token2".to_string()),
            AiEvent::Done(TokenBreakdown::default()),
        ];
        let mock = MockAiClientEvents::new(events);
        let (tx, mut rx) = mpsc::unbounded_channel::<AiEvent>();

        mock.chat("sys", vec![], tx, None).await.unwrap();

        match rx.try_recv().unwrap() {
            AiEvent::Token(t) => assert_eq!(t, "token1"),
            other => panic!("expected Token, got {:?}", other),
        }
        match rx.try_recv().unwrap() {
            AiEvent::Token(t) => assert_eq!(t, "token2"),
            other => panic!("expected Token, got {:?}", other),
        }
        match rx.try_recv().unwrap() {
            AiEvent::Done(_) => {}
            other => panic!("expected Done, got {:?}", other),
        }
        assert!(rx.try_recv().is_err(), "expected no more events");

        let calls = mock.calls();
        assert_eq!(calls.len(), 1);
    }
}
