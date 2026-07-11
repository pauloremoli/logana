pub mod session;
pub mod theme;
pub mod value_colors;
pub use theme::{
    Theme, ValueColorGroup, ValueColors, color_to_string, complete_theme, parse_color,
};
pub use value_colors::{
    VALUE_STYLE_HTTP_DELETE, VALUE_STYLE_HTTP_GET, VALUE_STYLE_HTTP_OTHER, VALUE_STYLE_HTTP_PATCH,
    VALUE_STYLE_HTTP_POST, VALUE_STYLE_HTTP_PUT, VALUE_STYLE_IP, VALUE_STYLE_STATUS_2XX,
    VALUE_STYLE_STATUS_3XX, VALUE_STYLE_STATUS_4XX, VALUE_STYLE_STATUS_5XX, VALUE_STYLE_UUID,
    collect_value_color_spans,
};

mod app;
mod commands;
pub mod field_layout;
mod input;
mod input_handler;
mod tabs;
pub use field_layout::FieldLayout;
mod render;
mod tab_state;
pub mod widgets;

pub use app::App;
pub use tab_state::merged::extend_merged_index;
pub use tab_state::{
    ArchiveExtractionState, ArchiveListingState, CacheState, CachedParsedLine, CachedScanResult,
    ConnectFn, DisplayConfig, FileLoadState, FileWatchState, FilterChunk, FilterEvalContext,
    FilterHandle, FilterState, FilterViewSnapshot, InteractionState, KeyResult, LoadContext,
    MergedState, ScrollState, SearchHandle, SearchState, SidebarSide, StdinLoadState,
    StreamConnection, StreamRetryState, StreamState, TabState, VisibleLines,
    apply_continuation_correction, build_continuation_map, display_text_for_line, dlt_connect_fn,
    docker_connect_fn, line_is_visible, merge_filter_counts, otlp_connect_fn, otlp_grpc_connect_fn,
    run_connect_fn, watch_state_from_connection, watch_state_from_file,
};
