use crate::filters::FilterType;
use crate::ui::App;

impl App {
    pub(super) fn cmd_tail(&mut self) {
        let tab = &mut self.tabs[self.active_tab];
        tab.stream.tail_mode = !tab.stream.tail_mode;
        if tab.stream.tail_mode {
            let new_count = tab.filter.visible_indices.len();
            tab.scroll.scroll_offset = new_count.saturating_sub(1);
        }
    }

    pub(super) fn cmd_stop(&mut self) {
        self.stdin_load_state = None;
        self.tabs[self.active_tab].stream.watch = None;
    }

    pub(super) fn cmd_pause(&mut self) {
        self.tabs[self.active_tab].stream.paused = true;
    }

    pub(super) fn cmd_resume(&mut self) {
        self.tabs[self.active_tab].stream.paused = false;
    }

    pub(super) async fn cmd_reset(&mut self) -> Result<bool, String> {
        self.db
            .reset_all()
            .await
            .map_err(|e| format!("Failed to reset database: {e}"))?;

        self.show_mode_bar = true;
        self.show_borders_default = false;
        self.show_line_numbers = true;
        self.show_sidebar = true;
        self.wrap = false;
        self.restore_policy = crate::config::RestoreSessionPolicy::Ask;
        self.restore_file_policy = crate::config::RestoreSessionPolicy::Ask;

        for tab in &mut self.tabs {
            tab.display.show_mode_bar = true;
            tab.display.show_borders = false;
            tab.display.show_line_numbers = true;
            tab.display.show_sidebar = true;
            tab.display.wrap = false;
            tab.reset_tab_state();
        }
        Ok(false)
    }

    pub(super) async fn cmd_date_filter(
        &mut self,
        expr: Vec<String>,
        fg: Option<String>,
        bg: Option<String>,
        line_mode: bool,
    ) -> Result<bool, String> {
        let tab = &self.tabs[self.active_tab];
        if tab.display.format.is_none() && !self.startup_filters {
            return Err(
                "No log format detected — date filter requires structured timestamps".to_string(),
            );
        }
        let expression = expr.join(" ");
        crate::filters::parse_date_filter(&expression)
            .map_err(|e| format!("Invalid date filter: {}", e))?;
        let pattern = format!("{}{}", crate::filters::DATE_PREFIX, expression);
        if let Some(old_id) = self.tabs[self.active_tab].filter.editing_filter_id.take() {
            self.tabs[self.active_tab]
                .log_manager
                .update_filter(
                    old_id,
                    pattern,
                    FilterType::Include,
                    fg.as_deref(),
                    bg.as_deref(),
                    !line_mode,
                )
                .await;
        } else {
            self.tabs[self.active_tab]
                .log_manager
                .add_filter_with_color(
                    pattern,
                    FilterType::Include,
                    fg.as_deref(),
                    bg.as_deref(),
                    !line_mode,
                )
                .await;
        }
        self.tabs[self.active_tab].begin_filter_refresh();
        Ok(false)
    }

    pub(super) fn cmd_docker(&mut self) -> Result<bool, String> {
        let output = std::process::Command::new("docker")
            .args([
                "ps",
                "--format",
                "{{.ID}}\t{{.Names}}\t{{.Image}}\t{{.Status}}",
            ])
            .output();
        match output {
            Ok(out) if out.status.success() => {
                let text = String::from_utf8_lossy(&out.stdout);
                let containers: Vec<crate::mode::docker_select_mode::DockerContainer> = text
                    .lines()
                    .filter(|l| !l.is_empty())
                    .filter_map(|line| {
                        let parts: Vec<&str> = line.splitn(4, '\t').collect();
                        if parts.len() == 4 {
                            Some(crate::mode::docker_select_mode::DockerContainer {
                                id: parts[0].to_string(),
                                name: parts[1].to_string(),
                                image: parts[2].to_string(),
                                status: parts[3].to_string(),
                            })
                        } else {
                            None
                        }
                    })
                    .collect();
                if containers.is_empty() {
                    self.tabs[self.active_tab].interaction.mode = Box::new(
                        crate::mode::docker_select_mode::DockerSelectMode::with_error(
                            "No running containers found".to_string(),
                        ),
                    );
                } else {
                    self.tabs[self.active_tab].interaction.mode = Box::new(
                        crate::mode::docker_select_mode::DockerSelectMode::new(containers),
                    );
                }
                Ok(true)
            }
            Ok(out) => {
                let stderr = String::from_utf8_lossy(&out.stderr).trim().to_string();
                self.tabs[self.active_tab].interaction.mode = Box::new(
                    crate::mode::docker_select_mode::DockerSelectMode::with_error(
                        if stderr.is_empty() {
                            "docker ps failed".to_string()
                        } else {
                            stderr
                        },
                    ),
                );
                Ok(true)
            }
            Err(e) => {
                self.tabs[self.active_tab].interaction.mode = Box::new(
                    crate::mode::docker_select_mode::DockerSelectMode::with_error(format!(
                        "Failed to run docker: {}",
                        e
                    )),
                );
                Ok(true)
            }
        }
    }

    pub(super) fn cmd_value_colors(&mut self) -> Result<bool, String> {
        use crate::mode::value_colors_mode::{ValueColorEntry, ValueColorGroup as VCGroup};
        let disabled = &self.theme.value_colors.disabled;
        let process_representative = self.theme.process_colors.first().copied();
        let groups: Vec<VCGroup> = self
            .theme
            .value_colors
            .grouped_categories(process_representative)
            .into_iter()
            .map(|g| VCGroup {
                label: g.label.to_string(),
                children: g
                    .children
                    .into_iter()
                    .map(|(key, label, color)| ValueColorEntry {
                        key: key.to_string(),
                        label: label.to_string(),
                        color,
                        enabled: !disabled.contains(key),
                    })
                    .collect(),
            })
            .collect();
        let original_disabled = disabled.clone();
        self.tabs[self.active_tab].interaction.mode = Box::new(
            crate::mode::value_colors_mode::ValueColorsMode::new(groups, original_disabled),
        );
        Ok(true)
    }

    pub(super) fn cmd_dlt(&mut self) -> Result<bool, String> {
        let devices = self.dlt_devices.clone();
        self.tabs[self.active_tab].interaction.mode =
            Box::new(crate::mode::dlt_select_mode::DltSelectMode::new(devices));
        Ok(true)
    }

    pub(super) async fn cmd_otel(&mut self, http: bool, port: Option<u16>) -> Result<bool, String> {
        if http {
            let port = port.unwrap_or(4318);
            self.open_otlp_stream(port).await;
        } else {
            let port = port.unwrap_or(4317);
            self.open_otlp_grpc_stream(port).await;
        }
        Ok(true)
    }

    pub(super) async fn cmd_enable_mcp(&mut self, port: u16) -> Result<bool, String> {
        let p = self.mcp_port.unwrap_or(port);
        match self.start_mcp(p).await {
            Ok(()) => {
                self.tabs[self.active_tab].interaction.notification =
                    Some(format!("MCP server listening on port {p}"));
                self.tabs[self.active_tab].interaction.notification_set_at =
                    Some(std::time::Instant::now());
            }
            Err(e) => {
                self.tabs[self.active_tab].interaction.command_error =
                    Some(format!("Failed to start MCP server on port {p}: {e}"));
            }
        }
        Ok(false)
    }

    pub(super) fn cmd_disable_mcp(&mut self) {
        self.stop_mcp();
        self.tabs[self.active_tab].interaction.notification =
            Some("MCP server stopped".to_string());
        self.tabs[self.active_tab].interaction.notification_set_at =
            Some(std::time::Instant::now());
    }
}
