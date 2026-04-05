use super::app::McpState;

impl McpState {
    pub fn stop(&mut self) {
        if let Some(handle) = self.server_handle.take() {
            handle.cancel.cancel();
        }
        self.cmd_rx = None;
    }

    pub async fn start(
        &mut self,
        port: u16,
        initial_snapshot: crate::mcp::McpSnapshot,
    ) -> std::io::Result<()> {
        self.stop();
        let (tx, rx) = tokio::sync::mpsc::channel(64);
        *self.snapshot.write().await = initial_snapshot;
        let handle = crate::mcp::start_mcp_server(port, self.snapshot.clone(), tx).await?;
        self.server_handle = Some(handle);
        self.cmd_rx = Some(rx);
        Ok(())
    }

    pub fn push_snapshot(&self, snapshot: crate::mcp::McpSnapshot) {
        let arc = self.snapshot.clone();
        tokio::spawn(async move {
            *arc.write().await = snapshot;
        });
    }

    pub fn drain_commands(&mut self) -> Vec<crate::mcp::McpCommand> {
        let mut cmds = Vec::new();
        if let Some(rx) = &mut self.cmd_rx {
            while let Ok(cmd) = rx.try_recv() {
                cmds.push(cmd);
            }
        }
        cmds
    }
}
