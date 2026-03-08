pub mod protocol;
pub mod server;
pub mod tools;

pub use server::{GraphUpdateEvent, GraphUpdateNotifier, McpServer, notification_channel};

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reexports_available() {
        // Verify key types are re-exported
        let _event = GraphUpdateEvent {
            files_changed: 0,
            node_count: 0,
            edge_count: 0,
        };
    }

    #[test]
    fn notification_channel_works() {
        let (tx, _rx) = notification_channel();
        let event = GraphUpdateEvent {
            files_changed: 1,
            node_count: 100,
            edge_count: 200,
        };
        // Should succeed (channel not full)
        let _ = tx.send(event);
    }

    #[test]
    fn mcp_server_constructible() {
        let graph = graphy_core::CodeGraph::new();
        let root = std::path::PathBuf::from("/tmp");
        let _server = McpServer::new(graph, None, root);
    }
}
