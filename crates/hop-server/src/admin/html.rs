use hop_core::{
    Asset, AssetAccessMode, AuthorizedKey, Credential, KnownHost, Session, ASSET_PRESET_MYSQL,
    ASSET_PRESET_POSTGRES, ASSET_PRESET_RDP, ASSET_PRESET_REDIS, ASSET_PRESET_VNC,
    ASSET_PROTOCOL_SSH, ASSET_PROTOCOL_TCP,
};
use maud::{html, Markup, PreEscaped, DOCTYPE};

use super::{
    i18n::{L10n, Locale},
    transfer::ImportSummary,
};

const ICON_OVERVIEW: &str = r#"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"><rect x="3" y="3" width="7" height="7" rx="1"/><rect x="14" y="3" width="7" height="7" rx="1"/><rect x="3" y="14" width="7" height="7" rx="1"/><rect x="14" y="14" width="7" height="7" rx="1"/></svg>"#;

const ICON_ASSETS: &str = r#"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"><rect x="2" y="2" width="20" height="8" rx="2"/><rect x="2" y="14" width="20" height="8" rx="2"/><circle cx="6" cy="6" r="1"/><circle cx="6" cy="18" r="1"/></svg>"#;

const ICON_CREDENTIALS: &str = r#"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"><path d="M21 2l-2 2m-7.61 7.61a5.5 5.5 0 1 1-7.778 7.778 5.5 5.5 0 0 1 7.777-7.777zm0 0L15.5 7.5m0 0l3 3L22 7l-3-3m-3.5 3.5L19 4"/></svg>"#;

const ICON_KEYS: &str = r#"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"><path d="M12 10a4 4 0 0 0-8 0c0 3 2 5.5 4 7.5C10 15.5 12 13 12 10z"/><path d="M12 10a4 4 0 0 1 8 0c0 3-2 5.5-4 7.5C14 15.5 12 13 12 10z"/><path d="M12 2v2"/><path d="M12 18v4"/></svg>"#;

const ICON_KNOWN_HOSTS: &str = r#"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"><path d="M12 22s8-4 8-10V5l-8-3-8 3v7c0 6 8 10 8 10z"/><path d="m9 12 2 2 4-4"/></svg>"#;

const ICON_SESSIONS: &str = r#"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"><polyline points="4 17 10 11 4 5"/><line x1="12" y1="19" x2="20" y2="19"/></svg>"#;

const ICON_IMPORT: &str = r#"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"><path d="M16 3h5v5"/><path d="M8 21H3v-5"/><path d="M21 3l-9 9"/><path d="M3 21l9-9"/></svg>"#;

const ICON_SETTINGS: &str = r#"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"><circle cx="12" cy="12" r="3"/><path d="M19.4 15a1.65 1.65 0 0 0 .33 1.82l.06.06a2 2 0 1 1-2.83 2.83l-.06-.06A1.65 1.65 0 0 0 15 19.4a1.65 1.65 0 0 0-1 .6 1.65 1.65 0 0 0-.33 1.82V22a2 2 0 1 1-4 0v-.09A1.65 1.65 0 0 0 8.6 20a1.65 1.65 0 0 0-1.82-.33l-.06.06a2 2 0 1 1-2.83-2.83l.06-.06A1.65 1.65 0 0 0 4.6 15a1.65 1.65 0 0 0-.6-1 1.65 1.65 0 0 0-1.82-.33H2a2 2 0 1 1 0-4h.09A1.65 1.65 0 0 0 4 8.6a1.65 1.65 0 0 0-.33-1.82l-.06-.06a2 2 0 1 1 2.83-2.83l.06.06A1.65 1.65 0 0 0 8.6 4a1.65 1.65 0 0 0 1-.6 1.65 1.65 0 0 0 .33-1.82V2a2 2 0 1 1 4 0v.09A1.65 1.65 0 0 0 15 4.6a1.65 1.65 0 0 0 1.82.33l.06-.06a2 2 0 1 1 2.83 2.83l-.06.06A1.65 1.65 0 0 0 19.4 8.6a1.65 1.65 0 0 0 .6 1 1.65 1.65 0 0 0 1.82.33H22a2 2 0 1 1 0 4h-.09A1.65 1.65 0 0 0 19.4 15z"/></svg>"#;

pub fn layout(title: &str, active: &str, t: &L10n, body_content: Markup) -> Markup {
    let alternate = t.locale.alternate();
    let language_href = language_switch_href(alternate, active);
    html! {
        (DOCTYPE)
        html lang=(t.locale.code()) {
            head {
                meta charset="utf-8";
                meta name="viewport" content="width=device-width, initial-scale=1";
                title { (title) " - " (t.app_title) }
                style {
                    r#"
                    :root {
                        color-scheme: dark;
                        --canvas: #0d1117;
                        --panel: #111827;
                        --panel-muted: #0f1724;
                        --field: #0a0f16;
                        --ink: #f9fafb;
                        --ink-soft: #cbd5e1;
                        --muted: #8b949e;
                        --border: #1f2937;
                        --border-strong: #263244;
                        --sidebar: #0a0f16;
                        --sidebar-panel: #0e1724;
                        --sidebar-ink: #f9fafb;
                        --sidebar-muted: #8b949e;
                        --control: #3b82f6;
                        --control-hover: #2563eb;
                        --secure: #22c55e;
                        --secure-soft: #0b2e1c;
                        --console-green: #22c55e;
                        --warn: #f59e0b;
                        --warn-soft: #312313;
                        --danger: #ef4444;
                        --danger-hover: #dc2626;
                        --danger-soft: #311216;
                        --row-hover: #101d2f;
                        --tag-bg: #122c4e;
                        --tag-ink: #bfdbfe;
                        --terminal: #060a10;
                        --shadow: 0 20px 54px rgba(0, 0, 0, 0.35);
                    }

                    * { box-sizing: border-box; }

                    body.admin-shell {
                        margin: 0;
                        min-height: 100vh;
                        background: var(--canvas);
                        color: var(--ink);
                        font-family: Inter, system-ui, sans-serif;
                        letter-spacing: 0;
                    }

                    a { color: inherit; }

                    a:focus-visible,
                    button:focus-visible,
                    input:focus-visible,
                    select:focus-visible,
                    textarea:focus-visible {
                        outline: 3px solid rgba(37, 99, 235, 0.28);
                        outline-offset: 2px;
                    }

                    .app-frame {
                        display: grid;
                        grid-template-columns: 272px minmax(0, 1fr);
                        min-height: 100vh;
                    }

                    .sidebar {
                        position: sticky;
                        top: 0;
                        height: 100svh;
                        padding: 22px 16px;
                        background: var(--sidebar);
                        border-right: 1px solid var(--border);
                        color: var(--sidebar-ink);
                        display: flex;
                        flex-direction: column;
                        gap: 24px;
                    }

                    .brand {
                        display: grid;
                        grid-template-columns: 40px minmax(0, 1fr);
                        gap: 12px;
                        align-items: center;
                        padding: 0 6px 14px;
                        border-bottom: 1px solid #263244;
                    }

                    .brand-mark {
                        width: 40px;
                        height: 40px;
                        border-radius: 8px;
                        display: grid;
                        place-items: center;
                        background: #122c4e;
                        border: 1px solid rgba(255, 255, 255, 0.16);
                        color: #bfdbfe;
                        font-weight: 850;
                        font-size: 1rem;
                        font-family: ui-monospace, SFMono-Regular, Menlo, monospace;
                    }

                    .brand strong {
                        display: block;
                        font-size: 1.02rem;
                        color: #ffffff;
                    }

                    .brand span {
                        color: var(--sidebar-muted);
                        font-size: 0.82rem;
                    }

                    .nav {
                        display: grid;
                        gap: 3px;
                    }

                    .nav-link {
                        position: relative;
                        min-height: 40px;
                        display: flex;
                        align-items: center;
                        gap: 10px;
                        padding: 9px 10px 9px 14px;
                        border-radius: 8px;
                        color: var(--sidebar-ink);
                        text-decoration: none;
                        font-weight: 650;
                        font-size: 0.9rem;
                        transition: background 160ms ease, color 160ms ease;
                    }

                    .nav-link svg {
                        width: 18px;
                        height: 18px;
                        opacity: 0.72;
                        flex-shrink: 0;
                        transition: opacity 160ms ease, color 160ms ease;
                    }

                    .nav-link:hover {
                        background: #0e1724;
                        color: #ffffff;
                    }

                    .nav-link:hover svg { opacity: 1; }

                    .nav-link.active {
                        background: #0f2a4a;
                        color: #ffffff;
                        box-shadow: inset 3px 0 0 var(--control);
                    }

                    .nav-link.active svg { opacity: 1; color: var(--control); }

                    .sidebar-footer {
                        margin-top: auto;
                        padding: 14px;
                        border: 1px solid #2d3a4d;
                        border-radius: 8px;
                        background: var(--sidebar-panel);
                        color: var(--sidebar-ink);
                        font-size: 0.84rem;
                    }

                    .sidebar-footer small {
                        display: block;
                        margin-top: 5px;
                        color: var(--sidebar-muted);
                        line-height: 1.45;
                    }

                    .status-dot {
                        width: 8px;
                        height: 8px;
                        display: inline-block;
                        margin-right: 8px;
                        border-radius: 999px;
                        background: #34d399;
                    }

                    .language-switch {
                        display: flex;
                        align-items: center;
                        justify-content: space-between;
                        gap: 10px;
                        margin-top: 12px;
                        padding-top: 12px;
                        border-top: 1px solid #2d3a4d;
                    }

                    .language-switch a {
                        color: #bfdbfe;
                        font-weight: 750;
                        text-decoration: none;
                    }

                    .language-switch a:hover { color: #ffffff; }

                    .content-shell {
                        min-width: 0;
                        background: var(--canvas);
                    }

                    .topbar {
                        position: sticky;
                        top: 0;
                        z-index: 10;
                        display: flex;
                        align-items: flex-start;
                        justify-content: space-between;
                        gap: 20px;
                        padding: 26px 38px 18px;
                        border-bottom: 1px solid var(--border);
                        background: rgba(13, 17, 23, 0.92);
                        backdrop-filter: blur(12px);
                    }

                    .eyebrow {
                        margin: 0 0 6px;
                        color: var(--control);
                        font-size: 0.76rem;
                        font-weight: 800;
                        text-transform: uppercase;
                        letter-spacing: 0;
                    }

                    .topbar h1 {
                        margin: 0;
                        font-size: 2rem;
                        line-height: 1.1;
                    }

                    .workspace {
                        width: min(1280px, 100%);
                        padding: 26px 38px 58px;
                    }

                    .page-intro {
                        display: grid;
                        gap: 7px;
                        margin-bottom: 20px;
                    }

                    .page-intro h2 {
                        margin: 0;
                        font-size: 1.2rem;
                    }

                    .page-intro p {
                        margin: 0;
                        color: var(--ink-soft);
                        max-width: 800px;
                        line-height: 1.58;
                    }

                    .panel {
                        margin: 0 0 18px;
                        padding: 20px;
                        border: 1px solid var(--border);
                        border-radius: 8px;
                        background: var(--panel);
                        box-shadow: var(--shadow);
                    }

                    .panel-header {
                        display: flex;
                        align-items: flex-start;
                        justify-content: space-between;
                        gap: 18px;
                        margin-bottom: 16px;
                    }

                    .panel-header h2 {
                        margin: 0;
                        font-size: 1.02rem;
                    }

                    .panel-header p {
                        margin: 5px 0 0;
                        color: var(--muted);
                        line-height: 1.5;
                    }

                    .grid {
                        display: grid;
                        grid-template-columns: repeat(auto-fit, minmax(230px, 1fr));
                        gap: 14px;
                    }

                    .field-wide { grid-column: 1 / -1; }

                    .button-row {
                        display: flex;
                        align-items: center;
                        flex-wrap: wrap;
                        gap: 9px;
                        margin-top: 16px;
                    }

                    .metric-grid {
                        display: grid;
                        grid-template-columns: repeat(auto-fit, minmax(190px, 1fr));
                        gap: 14px;
                        margin-bottom: 20px;
                    }

                    .metric {
                        min-height: 118px;
                        padding: 16px;
                        border: 1px solid var(--border);
                        border-radius: 8px;
                        background: var(--panel);
                        display: flex;
                        flex-direction: column;
                        justify-content: space-between;
                        box-shadow: var(--shadow);
                    }

                    .metric-label {
                        color: var(--muted);
                        font-size: 0.78rem;
                        font-weight: 780;
                        text-transform: uppercase;
                        letter-spacing: 0;
                    }

                    .metric-value {
                        font-size: 2.15rem;
                        line-height: 1;
                        font-weight: 850;
                        color: var(--ink);
                    }

                    .metric-note {
                        color: var(--ink-soft);
                        font-size: 0.84rem;
                    }

                    label.field {
                        display: grid;
                        gap: 7px;
                        color: var(--ink-soft);
                        font-size: 0.84rem;
                        font-weight: 720;
                    }

                    input, select, textarea {
                        width: 100%;
                        min-height: 40px;
                        padding: 9px 11px;
                        border: 1px solid var(--border-strong);
                        border-radius: 7px;
                        background: var(--field);
                        color: var(--ink);
                        font: inherit;
                        font-size: 0.94rem;
                        transition: border-color 140ms ease, box-shadow 140ms ease, background 140ms ease;
                    }

                    textarea {
                        min-height: 110px;
                        resize: vertical;
                        line-height: 1.45;
                    }

                    input:focus, select:focus, textarea:focus {
                        outline: 0;
                        border-color: var(--control);
                        background: #08111f;
                        box-shadow: 0 0 0 3px rgba(37, 99, 235, 0.12);
                    }

                    input[type=hidden] { display: none; }

                    input[type=checkbox] {
                        accent-color: var(--control);
                        cursor: pointer;
                    }

                    button, .button, .ghost-button {
                        min-height: 36px;
                        display: inline-flex;
                        align-items: center;
                        justify-content: center;
                        gap: 8px;
                        border-radius: 7px;
                        padding: 8px 12px;
                        font: inherit;
                        font-weight: 740;
                        font-size: 0.9rem;
                        text-decoration: none;
                        cursor: pointer;
                        transition: background 140ms ease, border-color 140ms ease, color 140ms ease, box-shadow 140ms ease;
                    }

                    button, .button {
                        border: 1px solid var(--control);
                        background: var(--control);
                        color: #ffffff;
                        box-shadow: 0 1px 2px rgba(15, 23, 42, 0.08);
                    }

                    button:hover, .button:hover {
                        background: var(--control-hover);
                        border-color: var(--control-hover);
                    }

                    .ghost-button {
                        border: 1px solid var(--border-strong);
                        background: #0a0f16;
                        color: var(--ink-soft);
                    }

                    .ghost-button:hover {
                        border-color: var(--control);
                        color: #bfdbfe;
                        background: #0f2a4a;
                    }

                    button.danger, .danger {
                        border-color: var(--danger);
                        background: var(--danger);
                        color: #ffffff;
                        box-shadow: none;
                    }

                    button.danger:hover, .danger:hover {
                        background: var(--danger-hover);
                        border-color: var(--danger-hover);
                    }

                    .muted, .fine-print {
                        color: var(--muted);
                        line-height: 1.55;
                    }

                    .error-message {
                        margin: 0 0 16px;
                        padding: 10px 12px;
                        border-radius: 7px;
                        background: var(--danger-soft);
                        color: var(--danger);
                        font-weight: 720;
                        border: 1px solid #fda29b;
                    }

                    .fine-print {
                        margin: 14px 0 0;
                        font-size: 0.86rem;
                    }

                    .table-wrap {
                        overflow-x: auto;
                        border: 1px solid var(--border);
                        border-radius: 8px;
                        background: var(--panel);
                    }

                    table.data-table {
                        width: 100%;
                        min-width: 760px;
                        border-collapse: collapse;
                    }

                    .data-table th,
                    .data-table td {
                        padding: 12px 14px;
                        border-bottom: 1px solid var(--border);
                        text-align: left;
                        vertical-align: top;
                    }

                    .data-table th {
                        color: #94a3b8;
                        background: var(--panel-muted);
                        font-size: 0.76rem;
                        font-weight: 820;
                        text-transform: uppercase;
                        letter-spacing: 0;
                    }

                    .data-table tr:last-child td { border-bottom: 0; }
                    .data-table tbody tr { transition: background 160ms ease, box-shadow 160ms ease; }
                    .data-table tbody tr:hover {
                        background: var(--row-hover);
                        box-shadow: inset 3px 0 0 var(--control);
                    }

                    .primary-cell {
                        display: grid;
                        gap: 4px;
                        font-weight: 760;
                    }

                    .subtle {
                        color: var(--muted);
                        font-size: 0.84rem;
                        font-weight: 500;
                    }

                    .mono {
                        font-family: ui-monospace, monospace;
                        font-size: 0.84rem;
                        line-height: 1.45;
                        word-break: break-all;
                        color: #93c5fd;
                    }

                    .tag-list, .secret-list, .action-row {
                        display: flex;
                        align-items: center;
                        flex-wrap: wrap;
                        gap: 7px;
                    }

                    .filter-row {
                        display: flex;
                        align-items: center;
                        flex-wrap: wrap;
                        gap: 8px;
                    }

                    .checkbox-cell {
                        width: 42px;
                        text-align: center;
                    }

                    .checkbox-cell input {
                        width: 18px;
                        min-height: 18px;
                    }

                    .asset-access-list {
                        display: grid;
                        gap: 10px;
                        margin-top: 14px;
                    }

                    .asset-access-item {
                        display: grid;
                        grid-template-columns: 22px minmax(0, 1fr);
                        gap: 10px;
                        align-items: start;
                        padding: 12px;
                        border: 1px solid var(--border);
                        border-radius: 8px;
                        background: var(--panel-muted);
                    }

                    .asset-access-item input {
                        width: 18px;
                        min-height: 18px;
                        margin-top: 2px;
                    }

                    .asset-access-item[hidden], [data-asset-access-list][hidden] {
                        display: none;
                    }

                    .import-summary {
                        display: grid;
                        gap: 8px;
                    }

                    .command-block {
                        display: grid;
                        gap: 8px;
                        margin-top: 8px;
                    }

                    .command-input {
                        font-family: ui-monospace, monospace;
                    }

                    .tag, .status-pill {
                        display: inline-flex;
                        align-items: center;
                        min-height: 24px;
                        border-radius: 999px;
                        padding: 3px 9px;
                        font-size: 0.78rem;
                        font-weight: 760;
                        white-space: nowrap;
                    }

                    .tag {
                        background: var(--tag-bg);
                        color: var(--tag-ink);
                        text-decoration: none;
                    }

                    .tag:hover {
                        background: #dbe7f6;
                        color: #17202a;
                    }

                    .status-pill {
                        background: var(--secure-soft);
                        color: var(--secure);
                    }

                    .status-pill.neutral {
                        background: var(--panel-muted);
                        color: #94a3b8;
                        border: 1px solid var(--border);
                    }

                    .status-pill.danger {
                        background: var(--danger-soft);
                        color: var(--danger);
                        border: 1px solid #fda29b;
                        box-shadow: none;
                    }

                    .action-row form { margin: 0; }

                    .empty-row td {
                        padding: 28px 14px;
                        color: var(--muted);
                        text-align: center;
                    }

                    .login-wrap {
                        max-width: 440px;
                        margin: 8vh auto 0;
                    }

                    pre {
                        white-space: pre-wrap;
                        border: 1px solid var(--border);
                        border-radius: 8px;
                        background: var(--terminal);
                        padding: 14px;
                        color: #bfdbfe;
                    }

                    .dashboard-page,
                    .assets-page,
                    .audit-page {
                        display: grid;
                        gap: 18px;
                    }

                    .console-hero {
                        display: flex;
                        align-items: flex-start;
                        justify-content: space-between;
                        gap: 18px;
                        padding: 2px 0 4px;
                    }

                    .console-hero h2 {
                        margin: 0;
                        font-size: 1.5rem;
                        line-height: 1.12;
                    }

                    .console-hero p {
                        margin: 7px 0 0;
                        color: var(--muted);
                        line-height: 1.5;
                    }

                    .console-actions,
                    .status-row {
                        display: flex;
                        align-items: center;
                        flex-wrap: wrap;
                        gap: 9px;
                    }

                    .status-chip,
                    .command-chip {
                        display: inline-flex;
                        align-items: center;
                        gap: 8px;
                        min-height: 28px;
                        padding: 5px 10px;
                        border-radius: 999px;
                        border: 1px solid var(--border);
                        background: #0a0f16;
                        color: var(--ink-soft);
                        font-size: 0.82rem;
                        font-weight: 760;
                        white-space: nowrap;
                    }

                    .status-chip.good {
                        background: var(--secure-soft);
                        border-color: #14532d;
                        color: var(--secure);
                    }

                    .status-chip.warn {
                        background: var(--warn-soft);
                        border-color: #713f12;
                        color: var(--warn);
                    }

                    .status-chip.danger {
                        background: var(--danger-soft);
                        border-color: #7f1d1d;
                        color: var(--danger);
                    }

                    .status-dot.good { background: var(--console-green); }
                    .status-dot.warn { background: var(--warn); }
                    .status-dot.danger { background: var(--danger); }

                    .dashboard-grid,
                    .audit-grid {
                        display: grid;
                        grid-template-columns: minmax(0, 1fr) 320px;
                        gap: 18px;
                        align-items: start;
                    }

                    .panel-stack {
                        display: grid;
                        gap: 18px;
                        min-width: 0;
                    }

                    .metric {
                        background: #111827;
                    }

                    .metric-value {
                        font-family: ui-monospace, SFMono-Regular, Menlo, monospace;
                    }

                    .metric-note strong {
                        color: var(--secure);
                        font-weight: 850;
                    }

                    .chart-bars {
                        height: 132px;
                        display: grid;
                        grid-template-columns: repeat(24, minmax(6px, 1fr));
                        align-items: end;
                        gap: 5px;
                    }

                    .chart-bars span {
                        min-height: 14px;
                        border-radius: 5px 5px 2px 2px;
                        background: linear-gradient(180deg, #60a5fa, #1d4ed8);
                        opacity: 0.86;
                    }

                    .heatmap {
                        display: grid;
                        grid-template-columns: repeat(12, 1fr);
                        gap: 5px;
                    }

                    .heatmap span {
                        aspect-ratio: 1;
                        min-width: 12px;
                        border-radius: 4px;
                        background: #0f1724;
                        border: 1px solid #172033;
                    }

                    .heatmap .level-1 { background: #0b2e1c; }
                    .heatmap .level-2 { background: #14532d; }
                    .heatmap .level-3 { background: #166534; }
                    .heatmap .level-4 { background: #22c55e; }

                    .posture-list,
                    .incident-list,
                    .feed-list {
                        display: grid;
                        gap: 10px;
                    }

                    .posture-item,
                    .incident-item,
                    .feed-item {
                        display: grid;
                        grid-template-columns: 10px minmax(0, 1fr) auto;
                        gap: 10px;
                        align-items: center;
                        padding: 10px 0;
                        border-bottom: 1px solid var(--border);
                    }

                    .posture-item:last-child,
                    .incident-item:last-child,
                    .feed-item:last-child {
                        border-bottom: 0;
                    }

                    .posture-item b,
                    .incident-item b,
                    .feed-item b {
                        color: var(--ink);
                        font-size: 0.9rem;
                    }

                    .posture-item span:last-child,
                    .incident-item span:last-child,
                    .feed-item span:last-child {
                        color: var(--muted);
                        font-size: 0.82rem;
                    }

                    .terminal-strip {
                        display: flex;
                        align-items: center;
                        gap: 10px;
                        padding: 12px 14px;
                        border: 1px solid var(--border);
                        border-radius: 8px;
                        background: var(--terminal);
                        color: #bfdbfe;
                        font-family: ui-monospace, SFMono-Regular, Menlo, monospace;
                        font-size: 0.84rem;
                        overflow-x: auto;
                    }

                    .assets-layout {
                        display: grid;
                        grid-template-columns: minmax(0, 1fr) 360px;
                        gap: 18px;
                        align-items: start;
                    }

                    .assets-layout > * {
                        min-width: 0;
                    }

                    .filter-console {
                        display: flex;
                        align-items: center;
                        justify-content: space-between;
                        gap: 12px;
                        flex-wrap: wrap;
                    }

                    .filter-console .filter-row {
                        flex: 1;
                        min-width: 260px;
                    }

                    .asset-form-panel {
                        position: sticky;
                        top: 106px;
                    }

                    .assets-page .panel-header {
                        flex-wrap: wrap;
                    }

                    .assets-page .data-table {
                        min-width: 660px;
                    }

                    .os-badge {
                        display: inline-grid;
                        place-items: center;
                        min-width: 34px;
                        min-height: 26px;
                        border-radius: 6px;
                        background: #122c4e;
                        color: #bfdbfe;
                        font-family: ui-monospace, SFMono-Regular, Menlo, monospace;
                        font-size: 0.72rem;
                        font-weight: 850;
                    }

                    .audit-toolbar {
                        display: flex;
                        align-items: center;
                        justify-content: space-between;
                        flex-wrap: wrap;
                        gap: 10px;
                        padding: 12px;
                        border: 1px solid var(--border);
                        border-radius: 8px;
                        background: #0a0f16;
                    }

                    .audit-event {
                        font-family: ui-monospace, SFMono-Regular, Menlo, monospace;
                        color: #bfdbfe;
                        font-size: 0.82rem;
                    }

                    .replay-box {
                        display: grid;
                        gap: 12px;
                        padding: 14px;
                        border: 1px solid var(--border);
                        border-radius: 8px;
                        background: var(--terminal);
                    }

                    .replay-progress {
                        height: 8px;
                        border-radius: 999px;
                        background: #111827;
                        overflow: hidden;
                    }

                    .replay-progress span {
                        display: block;
                        width: 46%;
                        height: 100%;
                        background: linear-gradient(90deg, #3b82f6, #22c55e);
                    }

                    .mobile-tabbar {
                        display: none;
                    }

                    @media (prefers-reduced-motion: reduce) {
                        *, *::before, *::after {
                            scroll-behavior: auto !important;
                            transition-duration: 0.01ms !important;
                            animation-duration: 0.01ms !important;
                            animation-iteration-count: 1 !important;
                        }
                    }

                    @media (max-width: 900px) {
                        .app-frame { grid-template-columns: 1fr; }
                        .sidebar {
                            display: none;
                        }
                        .topbar {
                            position: static;
                            padding: 22px 20px 16px;
                        }
                        .workspace {
                            padding: 22px 20px 96px;
                        }
                        .dashboard-grid,
                        .audit-grid,
                        .assets-layout {
                            grid-template-columns: 1fr;
                        }
                        .asset-form-panel {
                            position: static;
                        }
                        .mobile-tabbar {
                            position: fixed;
                            left: 0;
                            right: 0;
                            bottom: 0;
                            z-index: 20;
                            display: grid;
                            grid-template-columns: repeat(4, minmax(0, 1fr));
                            gap: 1px;
                            padding: 8px 10px calc(8px + env(safe-area-inset-bottom));
                            border-top: 1px solid var(--border);
                            background: rgba(10, 15, 22, 0.96);
                            backdrop-filter: blur(16px);
                        }
                        .mobile-tab {
                            min-height: 48px;
                            display: grid;
                            place-items: center;
                            gap: 3px;
                            border-radius: 8px;
                            color: var(--muted);
                            text-decoration: none;
                            font-size: 0.72rem;
                            font-weight: 760;
                        }
                        .mobile-tab svg {
                            width: 18px;
                            height: 18px;
                        }
                        .mobile-tab.active {
                            background: #0f2a4a;
                            color: #bfdbfe;
                        }
                    }

                    @media (max-width: 560px) {
                        .topbar {
                            flex-direction: column;
                            align-items: stretch;
                        }
                        .ghost-button, .button, button { width: 100%; }
                        .panel { padding: 16px; }
                        .panel-header { flex-direction: column; }
                        .topbar h1 { font-size: 1.65rem; }
                        .console-hero {
                            flex-direction: column;
                        }
                        .console-actions,
                        .status-row,
                        .filter-console,
                        .audit-toolbar {
                            align-items: stretch;
                            flex-direction: column;
                        }
                        .status-chip,
                        .command-chip {
                            justify-content: center;
                        }
                        .metric-grid {
                            grid-template-columns: repeat(2, minmax(0, 1fr));
                        }
                        .metric-value {
                            font-size: 1.8rem;
                        }
                        .heatmap {
                            grid-template-columns: repeat(8, 1fr);
                        }
                    }
                    "#
                }
            }
            body class="admin-shell" data-theme="operator" {
                div.app-frame {
                    aside.sidebar {
                        div.brand {
                            div.brand-mark { "H" }
                            div {
                                strong { "Hop" }
                                span { (t.admin_console) }
                            }
                        }
                        nav.nav aria-label=(t.nav_primary) {
                            (nav_link("/", t.nav_overview, ICON_OVERVIEW, active == "overview"))
                            (nav_link("/assets", t.nav_assets, ICON_ASSETS, active == "assets"))
                            (nav_link("/credentials", t.nav_credentials, ICON_CREDENTIALS, active == "credentials"))
                            (nav_link("/keys", t.nav_keys, ICON_KEYS, active == "keys"))
                            (nav_link("/known-hosts", t.nav_known_hosts, ICON_KNOWN_HOSTS, active == "known-hosts"))
                            (nav_link("/sessions", t.nav_sessions, ICON_SESSIONS, active == "sessions"))
                            (nav_link("/import", t.nav_import_export, ICON_IMPORT, active == "import"))
                            (nav_link("/settings", t.nav_settings, ICON_SETTINGS, active == "settings"))
                        }
                        div.sidebar-footer {
                            span.status-dot {}
                            (t.loopback_admin)
                            small { (t.loopback_note) }
                            div.language-switch {
                                span { (t.language_label) ": " (t.locale.label()) }
                                a href=(language_href) { (t.switch_language_to) " " (alternate.label()) }
                            }
                        }
                    }
                    div.content-shell {
                        header.topbar {
                            div {
                                p.eyebrow { (t.admin_web) }
                                h1 { (title) }
                            }
                            @if active != "login" {
                                a.ghost-button href="/logout" { (t.logout) }
                            }
                        }
                        main.workspace { (body_content) }
                        nav.mobile-tabbar aria-label=(t.nav_primary) {
                            (mobile_nav_link("/", t.nav_overview, ICON_OVERVIEW, active == "overview"))
                            (mobile_nav_link("/assets", t.nav_assets, ICON_ASSETS, active == "assets"))
                            (mobile_nav_link("/sessions", t.nav_sessions, ICON_SESSIONS, active == "sessions"))
                            (mobile_nav_link("/settings", t.nav_settings, ICON_SETTINGS, active == "settings"))
                        }
                    }
                }
            }
        }
    }
}

pub fn login(t: &L10n, error: Option<&str>) -> Markup {
    layout(
        t.login_title,
        "login",
        t,
        html! {
            div.login-wrap {
                section.panel {
                    div.panel-header {
                        div {
                            h2 { (t.login_heading) }
                            p { (t.login_intro) }
                        }
                    }
                    @if let Some(error) = error {
                        p.error-message { (error) }
                    }
                    form method="post" action="/login" {
                        label.field {
                            (t.login_password)
                            input type="password" name="password" required;
                        }
                        div.button-row {
                            button type="submit" { (t.login_button) }
                        }
                    }
                }
            }
        },
    )
}

pub fn settings(t: &L10n, csrf_token: &str, error: Option<&str>) -> Markup {
    layout(
        t.settings_title,
        "settings",
        t,
        html! {
            div.page-intro {
                h2 { (t.settings_heading) }
                p { (t.settings_intro) }
            }
            section.panel {
                div.panel-header {
                    div {
                        h2 { (t.admin_password_heading) }
                        p { (t.admin_password_intro) }
                    }
                }
                @if let Some(error) = error {
                    p.error-message { (error) }
                }
                form method="post" action="/settings" {
                    (csrf_field(csrf_token))
                    div.grid {
                        label.field {
                            (t.current_password)
                            input type="password" name="current_password" autocomplete="current-password" required;
                        }
                        label.field {
                            (t.new_password)
                            input type="password" name="new_password" autocomplete="new-password" required;
                        }
                        label.field {
                            (t.confirm_password)
                            input type="password" name="confirm_password" autocomplete="new-password" required;
                        }
                    }
                    div.button-row {
                        button type="submit" { (t.change_password) }
                    }
                }
            }
        },
    )
}

pub fn overview(
    t: &L10n,
    asset_count: usize,
    credential_count: usize,
    key_count: usize,
    session_count: usize,
) -> Markup {
    layout(
        t.overview_title,
        "overview",
        t,
        html! {
            div.dashboard-page {
                div.console-hero {
                    div {
                        h2 { (t.overview_heading) }
                        p { (t.overview_intro) }
                    }
                    div.console-actions {
                        span.status-chip.good { span.status-dot.good {} (t.overview_gateway_status) }
                        a.button href="/assets" { (t.assets_add_heading) }
                    }
                }
                div.metric-grid {
                    div.metric {
                        span.metric-label { (t.overview_assets_label) }
                        strong.metric-value { (asset_count) }
                        span.metric-note { (t.overview_assets_note) }
                    }
                    div.metric {
                        span.metric-label { (t.overview_sessions_label) }
                        strong.metric-value { (session_count) }
                        span.metric-note { (t.overview_sessions_note) }
                    }
                    div.metric {
                        span.metric-label { (t.overview_credentials_label) }
                        strong.metric-value { (credential_count) }
                        span.metric-note { (t.overview_credentials_note) }
                    }
                    div.metric {
                        span.metric-label { (t.overview_keys_label) }
                        strong.metric-value { (key_count) }
                        span.metric-note { (t.overview_keys_note) }
                    }
                }
                div.dashboard-grid {
                    div.panel-stack {
                        section.panel {
                            div.panel-header {
                                div {
                                    h2 { (t.overview_session_activity_heading) }
                                    p { (t.overview_session_activity_intro) }
                                }
                                span.status-chip.good { (session_count) " " (t.overview_recorded_suffix) }
                            }
                            div.terminal-strip {
                                span { "$" }
                                span { "hop admin sessions --limit 100" }
                            }
                        }
                    }
                    div.panel-stack {
                        section.panel {
                            div.panel-header {
                                div {
                                    h2 { (t.overview_inventory_health_heading) }
                                    p { (t.overview_inventory_health_intro) }
                                }
                            }
                            div.posture-list {
                                div.posture-item {
                                    span.status-dot.good {}
                                    b { (t.overview_assets_summary) }
                                    span { (asset_count) }
                                }
                                div.posture-item {
                                    span.status-dot.good {}
                                    b { (t.overview_credentials_summary) }
                                    span { (credential_count) }
                                }
                                div.posture-item {
                                    span.status-dot.good {}
                                    b { (t.overview_keys_summary) }
                                    span { (key_count) }
                                }
                            }
                        }
                        section.panel {
                            div.panel-header {
                                div {
                                    h2 { (t.overview_scope_heading) }
                                    p { (t.overview_scope_intro) }
                                }
                            }
                            p.fine-print { (t.overview_scope_note) }
                        }
                    }
                }
            }
        },
    )
}

pub fn assets(
    t: &L10n,
    items: &[Asset],
    credentials: &[Credential],
    csrf_token: &str,
    selected_tag: Option<&str>,
    all_tags: &[String],
    ssh_port: u16,
) -> Markup {
    layout(
        t.assets_title,
        "assets",
        t,
        html! {
            div.assets-page {
                div.console-hero {
                    div {
                        h2 { (t.assets_heading) }
                        p { (t.assets_intro) }
                    }
                    div.console-actions {
                        span.status-chip.good { span.status-dot.good {} (items.len()) " " (t.assets_count_suffix) }
                        a.button href="#add-asset" { (t.assets_add_heading) }
                    }
                }
                div.assets-layout {
                    div.panel-stack {
                        section.panel {
                            div.filter-console {
                                div {
                                    h2 { (t.assets_filter_heading) }
                                    p.fine-print { (t.assets_filter_intro) }
                                }
                                div.filter-row {
                                    a class=(if selected_tag.is_none() { "button" } else { "ghost-button" }) href="/assets" {
                                        (t.assets_filter_all)
                                    }
                                    @for tag in all_tags {
                                        a class=(if selected_tag == Some(tag.as_str()) { "button" } else { "ghost-button" })
                                          href=(format!("/assets?tag={}", url_query_value(tag))) {
                                            (tag)
                                        }
                                    }
                                }
                            }
                        }
                        section.panel {
                            div.panel-header {
                                div {
                                    h2 { (t.assets_existing_heading) }
                                    p { (t.assets_existing_intro) }
                                    p.fine-print { (t.assets_export_intro) }
                                }
                                div.status-row {
                                    span.status-chip.good { (items.len()) " " (t.assets_count_suffix) }
                                    span.command-chip { (t.assets_export_heading) }
                                    a.ghost-button href="/assets/export?format=csv" { (t.export_csv) }
                                    a.ghost-button href="/assets/export?format=json" { (t.export_json) }
                                    a.ghost-button href="/import" { (t.import_open) }
                                }
                            }
                            form method="post" action="/assets/bulk-tags" {
                                (csrf_field(csrf_token))
                                div.table-wrap {
                                    table.data-table {
                                        thead {
                                            tr {
                                                th.checkbox-cell {}
                                                th { (t.field_hostname) }
                                                th { (t.field_protocol) }
                                                th { (t.target_column) }
                                                th { (t.field_tags) }
                                                th { (t.field_credential) }
                                                th { (t.field_action) }
                                            }
                                        }
                                        tbody {
                                            @if items.is_empty() {
                                                tr.empty-row { td colspan="7" { (t.no_assets) } }
                                            }
                                            @for asset in items {
                                                tr {
                                                    td.checkbox-cell {
                                                        input type="checkbox" name="asset_ids" value=(asset.id);
                                                    }
                                                    td {
                                                        div.primary-cell {
                                                            (asset.name)
                                                            @if let Some(description) = &asset.description {
                                                                span.subtle { (description) }
                                                            } @else {
                                                                span.subtle { (t.asset_activity_placeholder) }
                                                            }
                                                            @if let Some(command) = asset_tunnel_command(asset, ssh_port) {
                                                                span.subtle.mono { (command) }
                                                            }
                                                        }
                                                    }
                                                    td { span.os-badge { (asset_protocol_label(t, asset_kind(asset))) } }
                                                    td.mono { (asset.hostname) ":" (asset.port) }
                                                    td {
                                                        div.tag-list {
                                                            @if asset.tags.is_empty() {
                                                                span.status-pill.neutral { (t.untagged) }
                                                            }
                                                            @for tag in &asset.tags {
                                                                a.tag href=(format!("/assets?tag={}", url_query_value(tag))) { (tag) }
                                                            }
                                                        }
                                                    }
                                                    td {
                                                        @if let Some(credential_id) = &asset.credential_id {
                                                            span.status-pill { (credential_id) }
                                                        } @else {
                                                            span.status-pill.neutral { (t.proxy_only) }
                                                        }
                                                    }
                                                    td {
                                                        div.action-row {
                                                            a class="button" href=(format!("/assets/{}/edit", asset.id)) { (t.edit) }
                                                            button class="danger" type="submit" formaction=(format!("/assets/{}/delete", asset.id)) { (t.delete) }
                                                        }
                                                    }
                                                }
                                            }
                                        }
                                    }
                                }
                                div.panel-header style="margin-top:18px;" {
                                    div {
                                        h2 { (t.assets_bulk_heading) }
                                        p { (t.assets_bulk_intro) }
                                    }
                                }
                                div.grid {
                                    label.field {
                                        (t.assets_bulk_tags_label)
                                        input name="tags" placeholder="prod, web" list="asset-tags-list";
                                    }
                                }
                                div.button-row {
                                    button type="submit" { (t.assets_bulk_apply) }
                                }
                            }
                        }
                    }
                    section.panel.asset-form-panel id="add-asset" {
                        div.panel-header {
                            div {
                                h2 { (t.assets_add_heading) }
                                p { (t.assets_add_intro) }
                            }
                            span.status-chip { (t.draft_status) }
                        }
                        form method="post" action="/assets" {
                            (csrf_field(csrf_token))
                            div.grid {
                                label.field {
                                    (t.field_name)
                                    input name="name" required;
                                }
                                label.field {
                                    (t.field_protocol)
                                    select name="protocol" onchange=(asset_protocol_onchange()) {
                                        (asset_protocol_options(t, ASSET_PROTOCOL_SSH))
                                    }
                                }
                                label.field {
                                    (t.field_hostname)
                                    input name="hostname" required;
                                }
                                label.field {
                                    (t.field_port)
                                    input name="port" type="number" value="22" required;
                                }
                                p class="fine-print field-wide" data-rdp-port-hint hidden { (t.rdp_port_hint) }
                                label.field {
                                    (t.field_tags)
                                    input name="tags" placeholder="prod, web" list="asset-tags-list";
                                }
                                label.field {
                                    (t.field_credential)
                                    select name="credential_id" {
                                        option value="" { (t.proxy_only) }
                                        @for credential in credentials {
                                            option value=(credential.id) { (credential.name) " (" (credential.username) ")" }
                                        }
                                    }
                                }
                                label.field.field-wide {
                                    (t.field_description)
                                    textarea name="description" {}
                                }
                            }
                            datalist id="asset-tags-list" {
                                @for tag in all_tags {
                                    option value=(tag) {}
                                }
                            }
                            div.terminal-strip {
                                span { "$" }
                                span { "ssh -p 22 hop@target.internal" }
                            }
                            div.button-row {
                                button type="submit" { (t.save_asset) }
                            }
                        }
                    }
                }
            }
        },
    )
}

pub fn edit_asset(
    t: &L10n,
    asset: &Asset,
    credentials: &[Credential],
    csrf_token: &str,
    all_tags: &[String],
    ssh_port: u16,
) -> Markup {
    layout(
        t.edit_asset_title,
        "assets",
        t,
        html! {
            div.page-intro {
                h2 { (asset.name) }
                p { (t.edit_asset_intro) }
            }
            section.panel {
                div.panel-header {
                    div {
                        h2 { (t.asset_details_heading) }
                        p { (t.asset_details_intro) }
                    }
                }
                form method="post" action=(format!("/assets/{}", asset.id)) {
                    (csrf_field(csrf_token))
                    div.grid {
                        label.field {
                            (t.field_name)
                            input name="name" value=(asset.name) required;
                        }
                        label.field {
                            (t.field_protocol)
                            select name="protocol" onchange=(asset_protocol_onchange()) {
                                (asset_protocol_options(t, asset_kind(asset)))
                            }
                        }
                        label.field {
                            (t.field_hostname)
                            input name="hostname" value=(asset.hostname) required;
                        }
                        label.field {
                            (t.field_port)
                            input name="port" type="number" value=(asset.port) required;
                        }
                        p class="fine-print field-wide" data-rdp-port-hint hidden[asset.preset.as_deref() != Some(ASSET_PRESET_RDP)] { (t.rdp_port_hint) }
                        label.field {
                            (t.field_tags)
                            input name="tags" value=(asset.tags.join(",")) placeholder="prod, web" list="asset-tags-list";
                        }
                        label.field {
                            (t.field_credential)
                            select name="credential_id" {
                                option value="" selected[asset.credential_id.is_none()] { (t.proxy_only) }
                                @for credential in credentials {
                                    option value=(credential.id) selected[asset.credential_id.as_deref() == Some(credential.id.as_str())] {
                                        (credential.name) " (" (credential.username) ")"
                                    }
                                }
                            }
                        }
                        label.field.field-wide {
                            (t.field_description)
                            textarea name="description" { (asset.description.as_deref().unwrap_or("")) }
                        }
                    }
                    datalist id="asset-tags-list" {
                        @for tag in all_tags {
                            option value=(tag) {}
                        }
                    }
                    div.button-row {
                        button type="submit" { (t.save_changes) }
                        a.ghost-button href="/assets" { (t.back_to_assets) }
                    }
                }
            }
            @if let Some(command) = asset_tunnel_command(asset, ssh_port) {
                section.panel {
                    div.panel-header {
                        div {
                            h2 { (t.tunnel_command_heading) }
                            p { (t.tunnel_command_intro) }
                        }
                    }
                    div.command-block {
                        input class="command-input" readonly value=(command);
                    }
                }
            }
        },
    )
}

fn asset_protocol_options(t: &L10n, selected: &str) -> Markup {
    html! {
        option value=(ASSET_PROTOCOL_SSH) selected[selected == ASSET_PROTOCOL_SSH] { (t.protocol_ssh) }
        option value=(ASSET_PROTOCOL_TCP) selected[selected == ASSET_PROTOCOL_TCP] { (t.protocol_tcp) }
        option value=(ASSET_PRESET_RDP) selected[selected == ASSET_PRESET_RDP] { (t.protocol_rdp) }
        option value=(ASSET_PRESET_VNC) selected[selected == ASSET_PRESET_VNC] { (t.protocol_vnc) }
        option value=(ASSET_PRESET_MYSQL) selected[selected == ASSET_PRESET_MYSQL] { (t.protocol_mysql) }
        option value=(ASSET_PRESET_POSTGRES) selected[selected == ASSET_PRESET_POSTGRES] { (t.protocol_postgres) }
        option value=(ASSET_PRESET_REDIS) selected[selected == ASSET_PRESET_REDIS] { (t.protocol_redis) }
    }
}

fn asset_protocol_onchange() -> &'static str {
    "const p=this.form.querySelector('[name=port]'); const d={ssh:22,tcp:22,rdp:3389,vnc:5900,mysql:3306,postgres:5432,redis:6379}; if(p&&d[this.value])p.value=d[this.value]; const h=this.form.querySelector('[data-rdp-port-hint]'); if(h)h.hidden=this.value!=='rdp';"
}

fn asset_protocol_label<'a>(t: &'a L10n, protocol: &'a str) -> &'a str {
    match protocol {
        ASSET_PROTOCOL_SSH => t.protocol_ssh,
        ASSET_PROTOCOL_TCP => t.protocol_tcp,
        ASSET_PRESET_RDP => t.protocol_rdp,
        ASSET_PRESET_VNC => t.protocol_vnc,
        ASSET_PRESET_MYSQL => t.protocol_mysql,
        ASSET_PRESET_POSTGRES => t.protocol_postgres,
        ASSET_PRESET_REDIS => t.protocol_redis,
        other => other,
    }
}

fn asset_kind(asset: &Asset) -> &str {
    asset.preset.as_deref().unwrap_or(&asset.protocol)
}

fn asset_tunnel_command(asset: &Asset, ssh_port: u16) -> Option<String> {
    if asset.protocol == ASSET_PROTOCOL_SSH {
        return None;
    }
    let local_port = match asset.preset.as_deref() {
        Some(ASSET_PRESET_RDP) => 13389,
        Some(ASSET_PRESET_VNC) => 15900,
        Some(ASSET_PRESET_MYSQL) => 13306,
        Some(ASSET_PRESET_POSTGRES) => 15432,
        Some(ASSET_PRESET_REDIS) => 16379,
        _ => asset.port,
    };
    Some(format!(
        "ssh -p {ssh_port} -N -T -L 127.0.0.1:{local_port}:{}:{} hop-host",
        asset_tunnel_target(asset),
        asset.port
    ))
}

fn asset_tunnel_target(asset: &Asset) -> String {
    if asset
        .name
        .chars()
        .all(|ch| ch.is_ascii_alphanumeric() || matches!(ch, '-' | '_' | '.'))
    {
        format!("{}.hop", asset.name)
    } else {
        asset.hostname.clone()
    }
}

pub fn credentials(t: &L10n, items: &[Credential], csrf_token: &str) -> Markup {
    layout(
        t.credentials_title,
        "credentials",
        t,
        html! {
            div.page-intro {
                h2 { (t.credentials_heading) }
                p { (t.credentials_intro) }
            }
            section.panel {
                div.panel-header {
                    div {
                        h2 { (t.credentials_export_heading) }
                        p { (t.credentials_export_intro) }
                    }
                }
                div.button-row {
                    a.button href="/credentials/export?format=csv" { (t.export_csv) }
                    a.button href="/credentials/export?format=json" { (t.export_json) }
                    a.ghost-button href="/import" { (t.import_open) }
                }
            }
            section.panel {
                div.panel-header {
                    div {
                        h2 { (t.credentials_add_heading) }
                        p { (t.credentials_add_intro) }
                    }
                }
                form method="post" action="/credentials" {
                    (csrf_field(csrf_token))
                    div.grid {
                        label.field {
                            (t.field_name)
                            input name="name" required;
                        }
                        label.field {
                            (t.field_username)
                            input name="username" required;
                        }
                        label.field {
                            (t.field_auth_type)
                            select name="auth_type" {
                                option value="password" { "password" }
                                option value="key" { "key" }
                                option value="key+passphrase" { "key+passphrase" }
                            }
                        }
                        label.field {
                            (t.field_password)
                            input type="password" name="password";
                        }
                        label.field {
                            (t.field_passphrase)
                            input type="password" name="passphrase";
                        }
                        label.field.field-wide {
                            (t.field_private_key)
                            textarea name="private_key" rows="8" {}
                        }
                    }
                    div.button-row {
                        button type="submit" { (t.save_credential) }
                    }
                    p.fine-print { (t.secret_storage_note) }
                }
            }
            section.panel {
                div.panel-header {
                    div {
                        h2 { (t.credentials_existing_heading) }
                        p { (t.credentials_existing_intro) }
                    }
                }
                div.table-wrap {
                    table.data-table {
                        thead {
                            tr { th { (t.field_name) } th { (t.field_username) } th { (t.field_auth_type) } th { (t.secrets_label) } th { (t.field_action) } }
                        }
                        tbody {
                            @if items.is_empty() {
                                tr.empty-row { td colspan="5" { (t.no_credentials) } }
                            }
                            @for credential in items {
                                tr {
                                    td {
                                        div.primary-cell {
                                            (credential.name)
                                            span.subtle.mono { (credential.id) }
                                        }
                                    }
                                    td { (credential.username) }
                                    td { span.status-pill { (credential.auth_type) } }
                                    td {
                                        div.secret-list {
                                            @if credential.password_enc.is_none() && credential.private_key_enc.is_none() && credential.passphrase_enc.is_none() {
                                                span.status-pill.neutral { (t.none) }
                                            }
                                            @if credential.password_enc.is_some() {
                                                span.tag { "password" }
                                            }
                                            @if credential.private_key_enc.is_some() {
                                                span.tag { "private key" }
                                            }
                                            @if credential.passphrase_enc.is_some() {
                                                span.tag { "passphrase" }
                                            }
                                        }
                                    }
                                    td {
                                        div.action-row {
                                            a class="button" href=(format!("/credentials/{}/edit", credential.id)) { (t.edit) }
                                            form method="post" action=(format!("/credentials/{}/delete", credential.id)) {
                                                (csrf_field(csrf_token))
                                                button class="danger" type="submit" { (t.delete) }
                                            }
                                        }
                                    }
                                }
                            }
                        }
                    }
                }
            }
        },
    )
}

pub fn edit_credential(t: &L10n, credential: &Credential, csrf_token: &str) -> Markup {
    layout(
        t.edit_credential_title,
        "credentials",
        t,
        html! {
            div.page-intro {
                h2 { (credential.name) }
                p { (t.edit_credential_intro) }
            }
            section.panel {
                div.panel-header {
                    div {
                        h2 { (t.credential_details_heading) }
                        p { (t.credential_details_intro) }
                    }
                }
                form method="post" action=(format!("/credentials/{}", credential.id)) {
                    (csrf_field(csrf_token))
                    div.grid {
                        label.field {
                            (t.field_name)
                            input name="name" value=(credential.name) required;
                        }
                        label.field {
                            (t.field_username)
                            input name="username" value=(credential.username) required;
                        }
                        label.field {
                            (t.field_auth_type)
                            select name="auth_type" {
                                option value="password" selected[credential.auth_type == "password"] { "password" }
                                option value="key" selected[credential.auth_type == "key"] { "key" }
                                option value="key+passphrase" selected[credential.auth_type == "key+passphrase"] { "key+passphrase" }
                            }
                        }
                        label.field {
                            (t.replace_password)
                            input type="password" name="password";
                        }
                        label.field {
                            (t.replace_passphrase)
                            input type="password" name="passphrase";
                        }
                        label.field.field-wide {
                            (t.replace_private_key)
                            textarea name="private_key" rows="8" {}
                        }
                    }
                    p.fine-print { (t.secret_keep_note) }
                    div.button-row {
                        button type="submit" { (t.save_changes) }
                        a.ghost-button href="/credentials" { (t.back_to_credentials) }
                    }
                }
            }
        },
    )
}

pub fn keys(t: &L10n, items: &[AuthorizedKey], csrf_token: &str) -> Markup {
    layout(
        t.keys_title,
        "keys",
        t,
        html! {
            div.page-intro {
                h2 { (t.keys_heading) }
                p { (t.keys_intro) }
            }
            section.panel {
                div.panel-header {
                    div {
                        h2 { (t.keys_add_heading) }
                        p { (t.keys_add_intro) }
                    }
                }
                form method="post" action="/keys" {
                    (csrf_field(csrf_token))
                    div.grid {
                        label.field {
                            (t.field_name)
                            input name="name" required;
                        }
                        label.field.field-wide {
                            (t.field_public_key)
                            textarea name="public_key" rows="4" required {}
                        }
                    }
                    div.button-row {
                        button type="submit" { (t.save_key) }
                    }
                }
            }
            section.panel {
                div.panel-header {
                    div {
                        h2 { (t.keys_existing_heading) }
                        p { (t.keys_existing_intro) }
                    }
                }
                div.table-wrap {
                    table.data-table {
                        thead {
                            tr { th { (t.field_name) } th { (t.field_fingerprint) } th { (t.field_status) } th { (t.field_action) } }
                        }
                        tbody {
                            @if items.is_empty() {
                                tr.empty-row { td colspan="4" { (t.no_keys) } }
                            }
                            @for key in items {
                                tr {
                                    td {
                                        div.primary-cell {
                                            (key.name)
                                            @if let Some(created_at) = &key.created_at {
                                                span.subtle { (t.key_added_prefix) " " (created_at) }
                                            }
                                        }
                                    }
                                    td.mono { (key.fingerprint) }
                                    td {
                                        @if key.is_active {
                                            span.status-pill { (t.active) }
                                        } @else {
                                            span.status-pill.neutral { (t.inactive) }
                                        }
                                    }
                                    td {
                                        div.action-row {
                                            a class="button" href=(format!("/keys/{}/edit", key.id)) { (t.edit) }
                                            @if key.is_active {
                                                form method="post" action=(format!("/keys/{}/deactivate", key.id)) {
                                                    (csrf_field(csrf_token))
                                                    button class="danger" type="submit" { (t.deactivate) }
                                                }
                                            } @else {
                                                form method="post" action=(format!("/keys/{}/activate", key.id)) {
                                                    (csrf_field(csrf_token))
                                                    button type="submit" { (t.activate) }
                                                }
                                            }
                                        }
                                    }
                                }
                            }
                        }
                    }
                }
            }
        },
    )
}

pub fn edit_key(
    t: &L10n,
    key: &AuthorizedKey,
    assets: &[Asset],
    assigned_ids: &[String],
    csrf_token: &str,
    error: Option<&str>,
) -> Markup {
    let restricted = key.asset_access_mode == AssetAccessMode::Restricted;
    let accessible_count = if restricted {
        assigned_ids.len()
    } else {
        assets.len()
    };
    layout(
        t.edit_key_title,
        "keys",
        t,
        html! {
            div.page-intro {
                h2 { (key.name) }
                p { (t.edit_key_intro) }
            }
            section.panel {
                div.panel-header {
                    div {
                        h2 { (t.key_details_heading) }
                        p { (t.key_details_intro) }
                    }
                }
                form method="post" action=(format!("/keys/{}", key.id)) {
                    (csrf_field(csrf_token))
                    @if let Some(error) = error {
                        p.error-message { (error) }
                    }
                    div.grid {
                        label.field {
                            (t.field_name)
                            input name="name" value=(key.name) required;
                        }
                        label.field.field-wide {
                            (t.field_public_key)
                            textarea name="public_key" rows="4" required { (key.public_key) }
                        }
                    }
                    section.asset-access-list {
                        div.panel-header {
                            div {
                                h2 { (t.key_access_heading) }
                                p { (t.key_access_intro) }
                            }
                            span.status-pill {
                                (accessible_count) " / " (assets.len()) " " (t.key_assets_suffix)
                            }
                        }
                        label.field {
                            (t.key_access_mode)
                            select name="asset_access_mode" data-asset-access-mode onchange=(key_access_mode_onchange()) {
                                option value="all" selected[!restricted] { (t.key_access_all) }
                                option value="restricted" selected[restricted] { (t.key_access_restricted) }
                            }
                        }
                        p.fine-print data-access-all-note hidden[restricted] { (t.key_access_all_intro) }
                        div data-asset-access-list hidden[!restricted] {
                            p.fine-print { (t.key_access_restricted_intro) }
                            label.field {
                                (t.key_asset_search)
                                input type="search" data-asset-filter oninput=(key_asset_filter_oninput());
                            }
                            div.asset-access-list {
                                @for asset in assets {
                                    @let assigned = assigned_ids.iter().any(|id| id == &asset.id);
                                    @let search = format!(
                                        "{} {} {} {} {}",
                                        asset.name,
                                        asset_kind(asset),
                                        asset.hostname,
                                        asset.port,
                                        asset.tags.join(" ")
                                    ).to_ascii_lowercase();
                                    label.asset-access-item data-asset-search=(search) {
                                        input type="checkbox" name="asset_id" value=(asset.id)
                                            checked[assigned] disabled[!restricted];
                                        div.primary-cell {
                                            span { (asset.name) }
                                            span.subtle {
                                                (asset_protocol_label(t, asset_kind(asset))) " · "
                                                (asset.hostname) ":" (asset.port)
                                            }
                                            @if !asset.tags.is_empty() {
                                                div.tag-list {
                                                    @for tag in &asset.tags {
                                                        span.tag { (tag) }
                                                    }
                                                }
                                            }
                                        }
                                    }
                                }
                            }
                        }
                    }
                    div.button-row {
                        button type="submit" { (t.save_changes) }
                        a.ghost-button href="/keys" { (t.back_to_keys) }
                    }
                }
            }
            script { (PreEscaped(key_access_script())) }
            section.panel {
                div.panel-header {
                    div {
                        h2 { (t.danger_zone) }
                        p { (t.delete_key_intro) }
                    }
                }
                form method="post" action=(format!("/keys/{}/delete", key.id)) {
                    (csrf_field(csrf_token))
                    button class="danger" type="submit" { (t.delete_key) }
                }
            }
        },
    )
}

fn key_access_mode_onchange() -> &'static str {
    "window.hopToggleKeyAccess(this.form)"
}

fn key_asset_filter_oninput() -> &'static str {
    "const q=this.value.toLowerCase();this.form.querySelectorAll('[data-asset-search]').forEach((row)=>row.hidden=!row.dataset.assetSearch.includes(q))"
}

fn key_access_script() -> &'static str {
    r#"window.hopToggleKeyAccess=function(form){const mode=form.querySelector('[data-asset-access-mode]').value;const restricted=mode==='restricted';const list=form.querySelector('[data-asset-access-list]');const note=form.querySelector('[data-access-all-note]');list.hidden=!restricted;note.hidden=restricted;list.querySelectorAll('input[type=checkbox]').forEach((input)=>input.disabled=!restricted);};"#
}

pub fn known_hosts(t: &L10n, items: &[KnownHost], csrf_token: &str) -> Markup {
    layout(
        t.known_hosts_title,
        "known-hosts",
        t,
        html! {
            div.page-intro {
                h2 { (t.known_hosts_heading) }
                p { (t.known_hosts_intro) }
            }
            section.panel {
                div.panel-header {
                    div {
                        h2 { (t.known_hosts_panel_heading) }
                        p { (t.known_hosts_panel_intro) }
                    }
                }
                div.table-wrap {
                    table.data-table {
                        thead {
                            tr { th { (t.host_column) } th { (t.key_type_column) } th { (t.field_fingerprint) } th { (t.first_seen_column) } th { (t.field_action) } }
                        }
                        tbody {
                            @if items.is_empty() {
                                tr.empty-row { td colspan="5" { (t.no_known_hosts) } }
                            }
                            @for host in items {
                                tr {
                                    td {
                                        div.primary-cell {
                                            (host.hostname)
                                            span.subtle.mono { ":" (host.port) }
                                        }
                                    }
                                    td { span.status-pill.neutral { (host.key_type) } }
                                    td.mono { (host.fingerprint) }
                                    td { (host.first_seen.as_deref().unwrap_or("-")) }
                                    td {
                                        div.action-row {
                                            form method="post" action=(format!("/known-hosts/{}/{}/delete", host.hostname, host.port)) {
                                                (csrf_field(csrf_token))
                                                input type="hidden" name="key_type" value=(host.key_type);
                                                button class="danger" type="submit" { (t.delete) }
                                            }
                                        }
                                    }
                                }
                            }
                        }
                    }
                }
            }
        },
    )
}

pub fn sessions(t: &L10n, items: &[Session]) -> Markup {
    layout(
        t.sessions_title,
        "sessions",
        t,
        html! {
            div.audit-page {
                div.console-hero {
                    div {
                        h2 { (t.sessions_heading) }
                        p { (t.sessions_intro) }
                    }
                    div.console-actions {
                        span.status-chip.danger { (items.iter().filter(|session| session.status == "failed").count()) " " (t.sessions_failed_suffix) }
                        span.status-chip.good { (items.len()) " " (t.sessions_recorded_suffix) }
                        a.ghost-button href="/sessions" { (t.sessions_live_tail) }
                    }
                }
                div.audit-toolbar {
                    div.terminal-strip {
                        span { "hop admin sessions --limit 100" }
                    }
                    div.status-row {
                        span.command-chip { (t.sessions_range_latest) }
                        span.command-chip { (t.sessions_user_all) }
                        span.command-chip { (t.sessions_event_all) }
                    }
                }
                div.audit-grid {
                    section.panel {
                        div.panel-header {
                            div {
                                h2 { (t.sessions_recent_heading) }
                                p { (t.sessions_recent_intro) }
                            }
                        }
                        div.table-wrap {
                            table.data-table {
                                thead {
                                    tr {
                                        th { (t.started_column) }
                                        th { (t.key_column) }
                                        th { (t.asset_column) }
                                        th { (t.mode_column) }
                                        th { (t.error_column) }
                                        th { (t.field_status) }
                                    }
                                }
                                tbody {
                                    @if items.is_empty() {
                                        tr.empty-row { td colspan="6" { (t.no_sessions) } }
                                    }
                                    @for session in items {
                                        tr {
                                            td.mono { (session.started_at.as_deref().unwrap_or("-")) }
                                            td {
                                                div.primary-cell {
                                                    (session.key_name.as_deref().unwrap_or("-"))
                                                    span.subtle.mono { (session.key_finger) }
                                                }
                                            }
                                            td { (session.asset_name.as_deref().unwrap_or("-")) }
                                            td { span.audit-event { (session_event_label(session)) } }
                                            td {
                                                div.primary-cell {
                                                    span.mono {
                                                        @if let Some(target_host) = &session.target_host {
                                                            (target_host) ":" (session.target_port.unwrap_or_default())
                                                        } @else {
                                                            "-"
                                                        }
                                                    }
                                                    @if let Some(client_ip) = &session.client_ip {
                                                        span.subtle { (t.sessions_source_prefix) " " (client_ip) }
                                                    }
                                                    @if let Some(error) = &session.error {
                                                        span.subtle { (error) }
                                                    }
                                                }
                                            }
                                            td {
                                                @if session.status == "failed" {
                                                    span.status-pill.danger { (session.status) }
                                                } @else if session.status == "ok" {
                                                    span.status-pill { (session.status) }
                                                } @else {
                                                    span.status-pill.neutral { (session.status) }
                                                }
                                            }
                                        }
                                    }
                                }
                            }
                        }
                    }
                    div.panel-stack {
                        section.panel {
                            div.panel-header {
                                div {
                                    h2 { (t.sessions_summary_heading) }
                                    p { (t.sessions_summary_intro) }
                                }
                            }
                            div.incident-list {
                                div.incident-item {
                                    span.status-dot.danger {}
                                    b { (t.sessions_failed_heading) }
                                    span { (items.iter().filter(|session| session.status == "failed").count()) }
                                }
                                div.incident-item {
                                    span.status-dot.warn {}
                                    b { (t.sessions_direct_heading) }
                                    span { (items.iter().filter(|session| session.mode == "direct").count()) }
                                }
                                div.incident-item {
                                    span.status-dot.good {}
                                    b { (t.sessions_completed_heading) }
                                    span { (items.iter().filter(|session| session.ended_at.is_some()).count()) }
                                }
                            }
                        }
                    }
                }
            }
        },
    )
}

fn session_event_label(session: &Session) -> &str {
    session.mode.as_str()
}

pub fn import_export(t: &L10n, csrf_token: &str, summary: Option<&ImportSummary>) -> Markup {
    layout(
        t.import_title,
        "import",
        t,
        html! {
            div.page-intro {
                h2 { (t.import_heading) }
                p { (t.import_intro) }
            }
            section.panel {
                div.panel-header {
                    div {
                        h2 { (t.import_form_heading) }
                        p { (t.import_form_intro) }
                    }
                }
                form method="post" action="/import" enctype="multipart/form-data" {
                    (csrf_field(csrf_token))
                    div.grid {
                        label.field {
                            (t.import_kind)
                            select name="kind" {
                                option value="assets" { (t.kind_assets) }
                                option value="credentials" { (t.kind_credentials) }
                            }
                        }
                        label.field {
                            (t.import_format)
                            select name="format" {
                                option value="csv" { "CSV" }
                                option value="json" { "JSON" }
                            }
                        }
                        label.field {
                            (t.import_conflict)
                            select name="on_conflict" {
                                option value="skip" { (t.conflict_skip) }
                                option value="overwrite" { (t.conflict_overwrite) }
                                option value="error" { (t.conflict_error) }
                            }
                        }
                        label.field.field-wide {
                            (t.import_file)
                            input type="file" name="file" required;
                        }
                    }
                    div.button-row {
                        button type="submit" { (t.import_submit) }
                    }
                }
            }
            @if let Some(summary) = summary {
                section.panel {
                    div.panel-header {
                        div {
                            h2 { (t.import_summary) }
                        }
                    }
                    div.import-summary {
                        p { (t.imported) ": " (summary.imported) }
                        p { (t.skipped) ": " (summary.skipped) }
                        p { (t.overwritten) ": " (summary.overwritten) }
                        @if !summary.errors.is_empty() {
                            p { (t.errors) ":" }
                            ul {
                                @for error in &summary.errors {
                                    li { (error) }
                                }
                            }
                        }
                    }
                }
            }
        },
    )
}

fn nav_link(href: &str, label: &str, icon: &str, active: bool) -> Markup {
    if active {
        html! {
            a class="nav-link active" href=(href) aria-current="page" {
                (PreEscaped(icon))
                (label)
            }
        }
    } else {
        html! {
            a class="nav-link" href=(href) {
                (PreEscaped(icon))
                (label)
            }
        }
    }
}

fn mobile_nav_link(href: &str, label: &str, icon: &str, active: bool) -> Markup {
    if active {
        html! {
            a class="mobile-tab active" href=(href) aria-current="page" {
                (PreEscaped(icon))
                span { (label) }
            }
        }
    } else {
        html! {
            a class="mobile-tab" href=(href) {
                (PreEscaped(icon))
                span { (label) }
            }
        }
    }
}

fn csrf_field(csrf_token: &str) -> Markup {
    html! {
        input type="hidden" name="csrf_token" value=(csrf_token);
    }
}

fn language_switch_href(locale: Locale, active: &str) -> String {
    format!(
        "/set-language?lang={}&redirect={}",
        locale.cookie_value(),
        url_query_value(active_path(active))
    )
}

fn active_path(active: &str) -> &'static str {
    match active {
        "overview" => "/",
        "assets" => "/assets",
        "credentials" => "/credentials",
        "keys" => "/keys",
        "known-hosts" => "/known-hosts",
        "sessions" => "/sessions",
        "import" => "/import",
        "settings" => "/settings",
        "login" => "/login",
        _ => "/",
    }
}

fn url_query_value(value: &str) -> String {
    let mut output = String::new();
    for byte in value.bytes() {
        match byte {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => {
                output.push(byte as char);
            }
            _ => {
                use std::fmt::Write as _;
                let _ = write!(&mut output, "%{byte:02X}");
            }
        }
    }
    output
}

#[cfg(test)]
mod tests {
    use super::super::i18n::{EN, ZH};
    use super::*;

    #[test]
    fn mutating_forms_include_csrf_token() {
        let rendered = assets(&EN, &[], &[], "csrf-123", None, &[], 2222).into_string();

        assert!(rendered.contains(r#"name="csrf_token""#));
        assert!(rendered.contains(r#"value="csrf-123""#));
    }

    #[test]
    fn layout_renders_admin_shell_and_active_navigation() {
        let rendered =
            layout(EN.assets_title, "assets", &EN, html! { p { "content" } }).into_string();

        assert!(rendered.contains(r#"class="admin-shell""#));
        assert!(rendered.contains(r#"aria-current="page""#));
        assert!(rendered.contains(r#"href="/assets""#));
        assert!(rendered.contains(r#"href="/settings""#));
        assert!(rendered.contains("/set-language?lang=zh"));
    }

    #[test]
    fn layout_uses_operator_console_theme_tokens() {
        let rendered =
            layout(EN.assets_title, "assets", &EN, html! { p { "content" } }).into_string();

        assert!(rendered.contains(r#"data-theme="operator""#));
        assert!(rendered.contains("--canvas: #0d1117"));
        assert!(rendered.contains("--sidebar: #0a0f16"));
        assert!(rendered.contains("--control: #3b82f6"));
        assert!(rendered.contains("--console-green: #22c55e"));
        assert!(rendered.contains("font-family: Inter, system-ui"));
        assert!(rendered.contains(".mobile-tabbar"));
        assert!(rendered.contains("@media (prefers-reduced-motion: reduce)"));
    }

    #[test]
    fn layout_css_does_not_emit_html_escaped_quotes() {
        let rendered =
            layout(EN.assets_title, "assets", &EN, html! { p { "content" } }).into_string();

        assert!(!rendered.contains("&quot;"));
    }

    #[test]
    fn layout_localizes_mobile_navigation_labels() {
        let rendered =
            layout(ZH.assets_title, "assets", &ZH, html! { p { "content" } }).into_string();

        assert!(rendered.contains("概览"));
        assert!(rendered.contains("资产"));
        assert!(rendered.contains("审计日志"));
        assert!(rendered.contains("设置"));
        assert!(!rendered.contains(">Dash<"));
        assert!(!rendered.contains(">Assets<"));
        assert!(!rendered.contains(">Audit<"));
        assert!(!rendered.contains(">Admin<"));
    }

    #[test]
    fn overview_renders_metric_tiles_with_labels() {
        let rendered = overview(&EN, 2, 3, 4, 5).into_string();

        assert!(rendered.contains(r#"class="dashboard-page""#));
        assert!(rendered.contains(r#"class="metric-grid""#));
        assert!(rendered.contains(r#"class="metric-value""#));
        assert!(rendered.contains("Bastion posture"));
        assert!(rendered.contains("Total servers"));
        assert!(rendered.contains("Recent sessions"));
        assert!(rendered.contains("5 recorded"));
        assert!(!rendered.contains("5 active"));
        assert!(!rendered.contains("Live Sessions"));
        assert!(!rendered.contains("Activity Heatmap"));
        assert!(!rendered.contains("JIT approvals pending"));
        assert!(!rendered.contains("Failed login watch"));
    }

    #[test]
    fn assets_page_renders_tag_filters_and_bulk_editor() {
        let tags = vec!["prod".to_string(), "web".to_string()];
        let rendered = assets(&EN, &[], &[], "csrf-123", Some("prod"), &tags, 2222).into_string();

        assert!(rendered.contains(r#"class="assets-page""#));
        assert!(rendered.contains("Inventory, connectivity, and assigned access tags."));
        assert!(rendered.contains("Server inventory"));
        assert!(rendered.contains("Add Asset"));
        assert!(rendered.contains(r#"href="/assets?tag=prod""#));
        assert!(rendered.contains(r#"action="/assets/bulk-tags""#));
        assert!(rendered.contains(r#"list="asset-tags-list""#));
    }

    #[test]
    fn sessions_page_renders_as_audit_replay_console() {
        let session_items = vec![Session {
            id: "session-1".to_string(),
            key_finger: "SHA256:test".to_string(),
            key_name: Some("alice".to_string()),
            mode: "direct".to_string(),
            asset_name: Some("prod-api-01".to_string()),
            target_host: Some("10.42.1.12".to_string()),
            target_port: Some(22),
            client_ip: Some("10.42.0.18".to_string()),
            status: "failed".to_string(),
            error: Some("password rejected".to_string()),
            started_at: Some("2026-06-17T14:39:12Z".to_string()),
            ended_at: None,
        }];
        let rendered = sessions(&EN, &session_items).into_string();

        assert!(rendered.contains(r#"class="audit-page""#));
        assert!(rendered.contains("Audit Logs"));
        assert!(rendered.contains("Forensic timeline"));
        assert!(rendered.contains("hop admin sessions --limit 100"));
        assert!(rendered.contains("direct"));
        assert!(rendered.contains("password rejected"));
        assert!(!rendered.contains("AUTH_FAIL"));
        assert!(!rendered.contains("Replay: latest SSH trace"));
        assert!(!rendered.contains("sudo systemctl reload postgres"));
        assert!(!rendered.contains("Policy Feed"));
    }

    #[test]
    fn assets_page_renders_protocol_controls_and_rdp_tunnel_hint() {
        let mut rdp = asset("win-rdp", "10.0.2.20", &["windows"]);
        rdp.protocol = ASSET_PROTOCOL_TCP.to_string();
        rdp.preset = Some(ASSET_PRESET_RDP.to_string());
        rdp.port = 3389;

        let rendered = assets(&EN, &[rdp], &[], "csrf-123", None, &[], 2222).into_string();

        assert!(rendered.contains(r#"name="protocol""#));
        assert!(rendered.contains(r#"value="rdp""#));
        assert!(rendered.contains("RDP"));
        assert!(rendered.contains(r#"data-rdp-port-hint"#));
        assert!(rendered.contains("3390"));
        assert!(rendered.contains("ssh -p 2222 -N -T -L 127.0.0.1:13389:win-rdp.hop:3389 hop-host"));
    }

    #[test]
    fn assets_page_renders_generic_tcp_presets_with_shared_tunnel_transport() {
        let cases = [
            (ASSET_PRESET_VNC, 5900, 15900),
            (ASSET_PRESET_MYSQL, 3306, 13306),
            (ASSET_PRESET_POSTGRES, 5432, 15432),
            (ASSET_PRESET_REDIS, 6379, 16379),
        ];

        for (preset, remote_port, local_port) in cases {
            let mut item = asset(preset, "10.0.0.20", &[]);
            item.protocol = ASSET_PROTOCOL_TCP.to_string();
            item.preset = Some(preset.to_string());
            item.port = remote_port;
            let rendered = assets(&EN, &[item], &[], "csrf-123", None, &[], 2222).into_string();

            assert!(rendered.contains(&format!(r#"value="{preset}""#)));
            assert!(rendered.contains(&format!(
                "ssh -p 2222 -N -T -L 127.0.0.1:{local_port}:{preset}.hop:{remote_port} hop-host"
            )));
        }
    }

    #[test]
    fn settings_page_renders_admin_password_form() {
        let rendered = settings(&EN, "csrf-123", Some("problem")).into_string();

        assert!(rendered.contains(r#"action="/settings""#));
        assert!(rendered.contains(r#"name="current_password""#));
        assert!(rendered.contains(r#"name="new_password""#));
        assert!(rendered.contains(r#"name="confirm_password""#));
        assert!(rendered.contains(r#"value="csrf-123""#));
        assert!(rendered.contains("problem"));
    }

    #[test]
    fn key_edit_page_renders_access_modes_search_and_assignments() {
        let mut key = authorized_key(AssetAccessMode::Restricted);
        let first = asset("first", "10.0.0.1", &["prod"]);
        let second = asset("second", "10.0.0.2", &[]);
        let rendered = edit_key(
            &EN,
            &key,
            &[first.clone(), second],
            std::slice::from_ref(&first.id),
            "csrf-123",
            Some("validation failed"),
        )
        .into_string();

        assert!(rendered.contains(r#"name="asset_access_mode""#));
        assert!(rendered.contains(r#"value="all""#));
        assert!(rendered.contains(r#"value="restricted" selected"#));
        assert!(rendered.contains(r#"type="search""#));
        assert!(rendered.contains(&format!(r#"value="{}" checked"#, first.id)));
        assert!(rendered.contains("1 / 2 assets"));
        assert!(rendered.contains("validation failed"));
        assert!(rendered.contains(r#"name="csrf_token" value="csrf-123""#));

        key.asset_access_mode = AssetAccessMode::All;
        let all_rendered = edit_key(&EN, &key, &[first], &[], "csrf-123", None).into_string();
        assert!(all_rendered.contains(r#"value="all" selected"#));
        assert!(all_rendered.contains("1 / 1 assets"));
        assert!(all_rendered.contains("Current and future assets are automatically accessible."));
    }

    #[test]
    fn import_page_uses_multipart_upload_form() {
        let rendered = import_export(&EN, "csrf-123", None).into_string();

        assert!(rendered.contains(r#"enctype="multipart/form-data""#));
        assert!(rendered.contains(r#"type="file""#));
    }

    fn asset(name: &str, hostname: &str, tags: &[&str]) -> Asset {
        Asset {
            id: name.to_string(),
            name: name.to_string(),
            protocol: ASSET_PROTOCOL_SSH.to_string(),
            preset: None,
            hostname: hostname.to_string(),
            port: 22,
            description: None,
            tags: tags.iter().map(|tag| tag.to_string()).collect(),
            credential_id: None,
            created_at: None,
            updated_at: None,
        }
    }

    fn authorized_key(mode: AssetAccessMode) -> AuthorizedKey {
        AuthorizedKey {
            id: "key-1".to_string(),
            name: "laptop".to_string(),
            public_key: "ssh-ed25519 AAAA".to_string(),
            fingerprint: "SHA256:test".to_string(),
            is_active: true,
            asset_access_mode: mode,
            created_at: None,
        }
    }
}
