// LlamaForge — core bundle: icons/atoms, app shell, shared components, a11y helpers.

/* ===== atoms.jsx ===== */
;(function(){
// Icon — Lucide-style stroked icons inline as SVG paths.
// Names: home, server, list, database, settings, terminal, box,
//        panelLeftClose, panelLeftOpen, helpCircle, chevronRight, search,
//        plus, refresh, activity, command.
const Icon = ({ name, size = 16, className = "" }) => {
  const paths = {
    home: <><path d="M3 11l9-8 9 8"/><path d="M5 10v10a1 1 0 0 0 1 1h4v-6h4v6h4a1 1 0 0 0 1-1V10"/></>,
    server: <><rect x="2" y="2" width="20" height="8" rx="2"/><rect x="2" y="14" width="20" height="8" rx="2"/><line x1="6" y1="6" x2="6.01" y2="6"/><line x1="6" y1="18" x2="6.01" y2="18"/></>,
    list: <><line x1="8" y1="6" x2="21" y2="6"/><line x1="8" y1="12" x2="21" y2="12"/><line x1="8" y1="18" x2="21" y2="18"/><line x1="3" y1="6" x2="3.01" y2="6"/><line x1="3" y1="12" x2="3.01" y2="12"/><line x1="3" y1="18" x2="3.01" y2="18"/></>,
    database: <><ellipse cx="12" cy="5" rx="9" ry="3"/><path d="M3 5v6c0 1.66 4 3 9 3s9-1.34 9-3V5"/><path d="M3 11v6c0 1.66 4 3 9 3s9-1.34 9-3v-6"/></>,
    settings: <><circle cx="12" cy="12" r="3"/><path d="M19.4 15a1.7 1.7 0 0 0 .3 1.8l.1.1a2 2 0 1 1-2.8 2.8l-.1-.1a1.7 1.7 0 0 0-1.8-.3 1.7 1.7 0 0 0-1 1.5V21a2 2 0 1 1-4 0v-.1a1.7 1.7 0 0 0-1.1-1.5 1.7 1.7 0 0 0-1.8.3l-.1.1a2 2 0 1 1-2.8-2.8l.1-.1a1.7 1.7 0 0 0 .3-1.8 1.7 1.7 0 0 0-1.5-1H3a2 2 0 1 1 0-4h.1a1.7 1.7 0 0 0 1.5-1.1 1.7 1.7 0 0 0-.3-1.8l-.1-.1a2 2 0 1 1 2.8-2.8l.1.1a1.7 1.7 0 0 0 1.8.3h0a1.7 1.7 0 0 0 1-1.5V3a2 2 0 1 1 4 0v.1a1.7 1.7 0 0 0 1 1.5h0a1.7 1.7 0 0 0 1.8-.3l.1-.1a2 2 0 1 1 2.8 2.8l-.1.1a1.7 1.7 0 0 0-.3 1.8v0a1.7 1.7 0 0 0 1.5 1H21a2 2 0 1 1 0 4h-.1a1.7 1.7 0 0 0-1.5 1z"/></>,
    terminal: <><polyline points="4 17 10 11 4 5"/><line x1="12" y1="19" x2="20" y2="19"/></>,
    box: <><path d="M21 16V8a2 2 0 0 0-1-1.73l-7-4a2 2 0 0 0-2 0l-7 4A2 2 0 0 0 3 8v8a2 2 0 0 0 1 1.73l7 4a2 2 0 0 0 2 0l7-4A2 2 0 0 0 21 16z"/><polyline points="3.27 6.96 12 12.01 20.73 6.96"/><line x1="12" y1="22.08" x2="12" y2="12"/></>,
    panelLeftClose: <><rect x="3" y="3" width="18" height="18" rx="2"/><line x1="9" y1="3" x2="9" y2="21"/><polyline points="16 9 13 12 16 15"/></>,
    panelLeftOpen: <><rect x="3" y="3" width="18" height="18" rx="2"/><line x1="9" y1="3" x2="9" y2="21"/><polyline points="13 9 16 12 13 15"/></>,
    helpCircle: <><circle cx="12" cy="12" r="10"/><path d="M9.09 9a3 3 0 0 1 5.83 1c0 2-3 3-3 3"/><line x1="12" y1="17" x2="12.01" y2="17"/></>,
    chevronRight: <polyline points="9 18 15 12 9 6"/>,
    search: <><circle cx="11" cy="11" r="8"/><line x1="21" y1="21" x2="16.65" y2="16.65"/></>,
    plus: <><line x1="12" y1="5" x2="12" y2="19"/><line x1="5" y1="12" x2="19" y2="12"/></>,
    refresh: <><path d="M21 12a9 9 0 1 1-3-6.7L21 8"/><polyline points="21 3 21 8 16 8"/></>,
    activity: <polyline points="22 12 18 12 15 21 9 3 6 12 2 12"/>,
    cpu: <><rect x="4" y="4" width="16" height="16" rx="2"/><rect x="9" y="9" width="6" height="6"/><line x1="9" y1="1" x2="9" y2="4"/><line x1="15" y1="1" x2="15" y2="4"/><line x1="9" y1="20" x2="9" y2="23"/><line x1="15" y1="20" x2="15" y2="23"/><line x1="20" y1="9" x2="23" y2="9"/><line x1="20" y1="14" x2="23" y2="14"/><line x1="1" y1="9" x2="4" y2="9"/><line x1="1" y1="14" x2="4" y2="14"/></>,
    zap: <polygon points="13 2 3 14 12 14 11 22 21 10 12 10 13 2"/>,
    gauge: <><path d="M12 14l4-4"/><path d="M3.34 19a10 10 0 1 1 17.32 0"/><circle cx="12" cy="14" r="1.5" fill="currentColor" stroke="none"/></>,
    volume: <><polygon points="11 5 6 9 2 9 2 15 6 15 11 19 11 5"/><path d="M15.54 8.46a5 5 0 0 1 0 7.07"/></>,
    volume2: <><polygon points="11 5 6 9 2 9 2 15 6 15 11 19 11 5"/><path d="M15.54 8.46a5 5 0 0 1 0 7.07"/><path d="M19.07 4.93a10 10 0 0 1 0 14.14"/></>,
    serverPlus: <><rect x="2" y="3" width="14" height="7" rx="2"/><rect x="2" y="14" width="14" height="7" rx="2"/><line x1="6" y1="6.5" x2="6.01" y2="6.5"/><line x1="6" y1="17.5" x2="6.01" y2="17.5"/><line x1="20" y1="6" x2="20" y2="12"/><line x1="17" y1="9" x2="23" y2="9"/></>,
    serverOff: <><path d="M2 8a2 2 0 0 1 2-2h12"/><path d="M22 10V8"/><path d="M6 6 2 2"/><rect x="2" y="14" width="20" height="7" rx="2"/><line x1="6" y1="17.5" x2="6.01" y2="17.5"/><line x1="2" y1="2" x2="22" y2="22"/></>,
    clock: <><circle cx="12" cy="12" r="9"/><polyline points="12 7 12 12 15.5 14"/></>,
    alertTriangle: <><path d="M10.29 3.86 1.82 18a2 2 0 0 0 1.71 3h16.94a2 2 0 0 0 1.71-3L13.71 3.86a2 2 0 0 0-3.42 0z"/><line x1="12" y1="9" x2="12" y2="13"/><line x1="12" y1="17" x2="12.01" y2="17"/></>,
    plugZap: <><path d="M6.3 20.3a2.4 2.4 0 0 0 3.4 0L12 18l-6-6-2.3 2.3a2.4 2.4 0 0 0 0 3.4Z"/><path d="m2 22 3-3"/><path d="M7.5 13.5 10 11"/><path d="M10.5 16.5 13 14"/><path d="m18 3-4 4h6l-4 4"/></>,
    power: <><path d="M12 2v10"/><path d="M18.36 6.64a9 9 0 1 1-12.73 0"/></>,
    square: <rect x="5" y="5" width="14" height="14" rx="1"/>,
    xOctagon: <><polygon points="7.86 2 16.14 2 22 7.86 22 16.14 16.14 22 7.86 22 2 16.14 2 7.86 7.86 2"/><line x1="15" y1="9" x2="9" y2="15"/><line x1="9" y1="9" x2="15" y2="15"/></>,
    edit: <><path d="M11 4H4a2 2 0 0 0-2 2v14a2 2 0 0 0 2 2h14a2 2 0 0 0 2-2v-7"/><path d="M18.5 2.5a2.12 2.12 0 0 1 3 3L12 15l-4 1 1-4 9.5-9.5z"/></>,
    externalLink: <><path d="M18 13v6a2 2 0 0 1-2 2H5a2 2 0 0 1-2-2V8a2 2 0 0 1 2-2h6"/><polyline points="15 3 21 3 21 9"/><line x1="10" y1="14" x2="21" y2="3"/></>,
    check: <polyline points="20 6 9 17 4 12"/>,
    checkCircle: <><path d="M22 11.08V12a10 10 0 1 1-5.93-9.14"/><polyline points="22 4 12 14.01 9 11.01"/></>,
    chevronDown: <polyline points="6 9 12 15 18 9"/>,
    chevronUp: <polyline points="18 15 12 9 6 15"/>,
    arrowUp: <><line x1="12" y1="19" x2="12" y2="5"/><polyline points="5 12 12 5 19 12"/></>,
    arrowDown: <><line x1="12" y1="5" x2="12" y2="19"/><polyline points="19 12 12 19 5 12"/></>,
    menu: <><line x1="3" y1="12" x2="21" y2="12"/><line x1="3" y1="6" x2="21" y2="6"/><line x1="3" y1="18" x2="21" y2="18"/></>,
    loader: <><line x1="12" y1="2" x2="12" y2="6"/><line x1="12" y1="18" x2="12" y2="22"/><line x1="4.93" y1="4.93" x2="7.76" y2="7.76"/><line x1="16.24" y1="16.24" x2="19.07" y2="19.07"/><line x1="2" y1="12" x2="6" y2="12"/><line x1="18" y1="12" x2="22" y2="12"/><line x1="4.93" y1="19.07" x2="7.76" y2="16.24"/><line x1="16.24" y1="7.76" x2="19.07" y2="4.93"/></>,
    x: <><line x1="18" y1="6" x2="6" y2="18"/><line x1="6" y1="6" x2="18" y2="18"/></>,
    download: <><path d="M21 15v4a2 2 0 0 1-2 2H5a2 2 0 0 1-2-2v-4"/><polyline points="7 10 12 15 17 10"/><line x1="12" y1="15" x2="12" y2="3"/></>,
    sliders: <><line x1="4" y1="21" x2="4" y2="14"/><line x1="4" y1="10" x2="4" y2="3"/><line x1="12" y1="21" x2="12" y2="12"/><line x1="12" y1="8" x2="12" y2="3"/><line x1="20" y1="21" x2="20" y2="16"/><line x1="20" y1="12" x2="20" y2="3"/><line x1="1" y1="14" x2="7" y2="14"/><line x1="9" y1="8" x2="15" y2="8"/><line x1="17" y1="16" x2="23" y2="16"/></>,
    lock: <><rect x="3" y="11" width="18" height="11" rx="2"/><path d="M7 11V7a5 5 0 0 1 10 0v4"/></>,
    info: <><circle cx="12" cy="12" r="10"/><line x1="12" y1="16" x2="12" y2="12"/><line x1="12" y1="8" x2="12.01" y2="8"/></>,
    alertCircle: <><circle cx="12" cy="12" r="10"/><line x1="12" y1="8" x2="12" y2="12"/><line x1="12" y1="16" x2="12.01" y2="16"/></>,
    fileText: <><path d="M14 2H6a2 2 0 0 0-2 2v16a2 2 0 0 0 2 2h12a2 2 0 0 0 2-2V8z"/><polyline points="14 2 14 8 20 8"/><line x1="16" y1="13" x2="8" y2="13"/><line x1="16" y1="17" x2="8" y2="17"/><line x1="10" y1="9" x2="8" y2="9"/></>,
    bookOpen: <><path d="M2 3h6a4 4 0 0 1 4 4v14a3 3 0 0 0-3-3H2z"/><path d="M22 3h-6a4 4 0 0 0-4 4v14a3 3 0 0 1 3-3h7z"/></>,
  };
  return (
    <svg className={`icon ${className}`} viewBox="0 0 24 24" width={size} height={size}
         fill="none" stroke="currentColor" strokeWidth="1.75"
         strokeLinecap="round" strokeLinejoin="round" aria-hidden="true">
      {paths[name] || null}
    </svg>
  );
};

// LF monogram — copied logo concept, scaled to 22px box
const LFMark = ({ size = 22 }) => (
  <svg width={size} height={size} viewBox="0 0 32 32" aria-hidden="true">
    <rect width="32" height="32" rx="6" fill="var(--surface-inverse)"/>
    <text x="3.5" y="23" fontFamily="var(--font-sans)" fontWeight="700" fontSize="18" letterSpacing="-0.04em" fill="var(--text-inverse)">L</text>
    <rect x="15" y="7" width="1.4" height="18" fill="var(--accent-blue)"/>
    <text x="17.5" y="23" fontFamily="var(--font-sans)" fontWeight="700" fontSize="18" letterSpacing="-0.04em" fill="var(--text-inverse)">F</text>
  </svg>
);

// StatusBadge — same as design system
const STATUS_GLYPH = { healthy: "✓", degraded: "⚠", failed: "✕", pending: "⟳", cancelled: "⊘", timeout: "⧖" };
const STATUS_LABEL = { healthy: "Healthy", degraded: "Degraded", failed: "Failed", pending: "Restarting…", cancelled: "Cancelled", timeout: "Timeout" };
const StatusBadge = ({ status, label, sm }) => (
  <span className={`StatusBadge ${status} ${sm ? "sm" : ""}`}>
    <span className="g">{STATUS_GLYPH[status]}</span>
    <span>{label || STATUS_LABEL[status]}</span>
  </span>
);

// Sparkline
const Sparkline = ({ points, color = "var(--accent-blue)", filled = true, height = 32 }) => {
  const W = 200, H = 32;
  const xs = points.map((_, i) => (i / (points.length - 1)) * W);
  const max = Math.max(...points), min = Math.min(...points);
  const range = Math.max(max - min, 0.0001);
  const ys = points.map(p => H - 3 - ((p - min) / range) * (H - 7));
  const d = xs.map((x, i) => `${i === 0 ? "M" : "L"}${x.toFixed(1)},${ys[i].toFixed(1)}`).join(" ");
  const area = `${d} L${W},${H} L0,${H} Z`;
  return (
    <svg className="spark-svg" viewBox={`0 0 ${W} ${H}`} preserveAspectRatio="none" style={{ height }}>
      {filled && <path d={area} fill={color} fillOpacity="0.10"/>}
      <path d={d} fill="none" stroke={color} strokeWidth="1.5" />
    </svg>
  );
};

Object.assign(window, { Icon, LFMark, StatusBadge, Sparkline });

})();

/* ===== app-shell.jsx ===== */
;(function(){
// AppShell — LlamaForge top-level layout.
// Renders sidebar + topbar + page area. Used inside every artboard.

const NAV_ITEMS = [
  { id: "dashboard",     label: "Dashboard",     icon: "home" },
  { id: "servers",       label: "Servers",       icon: "server",    count: 7 },
  { id: "queue",         label: "Queue",         icon: "list",      count: 12 },
  { id: "explorer",      label: "Explorer",      icon: "database" },
  { id: "configuration", label: "Configuration", icon: "settings" },
  { id: "playground",    label: "Playground",    icon: "terminal" },
  { id: "models",        label: "Models",        icon: "box",       count: 4 },
];

// ─── Sidebar ─────────────────────────────────────────────────────────
const Sidebar = ({ activeId, onNav, collapsed, onToggle, hideCounts }) => (
  <aside className="Sidebar" aria-label="Primary">
    <div className="brand">
      <span className="mark"><LFMark size={22} /></span>
      <span className="word">LlamaForge</span>
      <span className="ver">v0.1.0</span>
    </div>
    <nav>
      {NAV_ITEMS.map((it) => {
        const active = it.id === activeId;
        const count = hideCounts ? null : it.count;
        return (
          <button
            key={it.id}
            className="NavItem"
            aria-current={active ? "page" : undefined}
            onClick={() => onNav && onNav(it.id)}
            title={collapsed ? it.label : undefined}
          >
            <Icon name={it.icon} size={16} />
            <span className="label">{it.label}</span>
            {count != null && !collapsed && (
              <span style={{
                marginLeft: "auto",
                font: "11px var(--font-mono)",
                color: active ? "var(--accent-blue)" : "var(--text-tertiary)"
              }}>{count}</span>
            )}
            <span className="tip">{it.label}</span>
          </button>
        );
      })}
    </nav>
    <div className="foot">
      <button
        className="CollapseBtn"
        onClick={onToggle}
        aria-label={collapsed ? "Expand sidebar" : "Collapse sidebar"}
        aria-expanded={!collapsed}
        title={collapsed ? "Expand sidebar" : "Collapse sidebar"}
      >
        <Icon name={collapsed ? "panelLeftOpen" : "panelLeftClose"} size={16} />
        {!collapsed && <span className="lbl">Collapse</span>}
        {!collapsed && <span className="kbd-mini">⌘\</span>}
      </button>
    </div>
  </aside>
);

// ─── TopBar ──────────────────────────────────────────────────────────
const Breadcrumb = ({ crumbs }) => (
  <div className="crumbs">
    {crumbs.map((c, i) => (
      <React.Fragment key={i}>
        {i > 0 && (
          <svg className="sep" viewBox="0 0 24 24" width="14" height="14" aria-hidden="true">
            <polyline points="9 18 15 12 9 6"/>
          </svg>
        )}
        <span className={`crumb ${i === crumbs.length - 1 ? "tail" : ""} ${c.mono ? "mono" : ""}`}>
          {c.label}
        </span>
      </React.Fragment>
    ))}
  </div>
);

const ConnBadge = ({ state, meta }) => {
  const label = state === "connected" ? "Connected"
              : state === "reconnecting" ? "Reconnecting…"
              : "Disconnected";
  return (
    <span className={`ConnBadge ${state}`} role="status" aria-live="polite">
      <span className="dot" aria-hidden="true"/>
      <span>{label}</span>
      {meta && <span className="meta">{meta}</span>}
    </span>
  );
};

const TopBar = ({ crumbs, conn = "connected", connMeta }) => (
  <header className="TopBar">
    <Breadcrumb crumbs={crumbs} />
    <div className="spacer" />
    <ConnBadge state={conn} meta={connMeta} />
    <button className="IconBtn" aria-label="Keyboard shortcuts" title="Keyboard shortcuts (?)">
      <span className="qmark">?</span>
    </button>
  </header>
);

// ─── Connection banner (below header) ────────────────────────────────
const Banner = ({ kind, meta }) => {
  if (kind === "warn") {
    return (
      <div className="Banner warn" role="status" aria-live="polite">
        <Icon name="refresh" size={16} className="bi" />
        <span className="btext">Live updates paused — reconnecting…</span>
        <span className="bmeta">Last updated: {meta || "12s ago"}</span>
        <button className="baction">Retry now</button>
      </div>
    );
  }
  if (kind === "error") {
    return (
      <div className="Banner error" role="alert">
        <svg className="bi" viewBox="0 0 24 24" width="16" height="16" fill="none"
             stroke="currentColor" strokeWidth="1.75" strokeLinecap="round" strokeLinejoin="round" aria-hidden="true">
          <path d="M10.29 3.86 1.82 18a2 2 0 0 0 1.71 3h16.94a2 2 0 0 0 1.71-3L13.71 3.86a2 2 0 0 0-3.42 0z"/>
          <line x1="12" y1="9" x2="12" y2="13"/><line x1="12" y1="17" x2="12.01" y2="17"/>
        </svg>
        <span className="btext">No network connection</span>
        <span className="bmeta">Last updated: {meta || "1m 40s ago"}</span>
        <button className="baction">Retry now</button>
      </div>
    );
  }
  return null;
};

// ─── Sample Dashboard content ────────────────────────────────────────
const MetricCard = ({ title, value, unit, delta, deltaKind, icon, spark, sparkColor }) => (
  <div className="MetricCard">
    <div className="title">
      <span>{title}</span>
      <Icon name={icon} size={14} />
    </div>
    <div className="value">{value}{unit && <span className="u">{unit}</span>}</div>
    {delta && (
      <div className={`delta ${deltaKind || ""}`}>{delta}</div>
    )}
    {spark && <div className="spark"><Sparkline points={spark} color={sparkColor} /></div>}
  </div>
);

const FleetRow = ({ id, gpu, status, slots, tps, kv }) => (
  <div className="FleetRow" role="row">
    <div className="name">
      <span className="id">{id}</span>
      <span className="gpu">{gpu}</span>
    </div>
    <StatusBadge status={status} />
    <div className="slots" aria-label={`slots ${slots.filter(s=>s).length}/${slots.length}`}>
      {slots.map((s, i) => <i key={i} className={s || ""} />)}
    </div>
    <div className="num">{tps}<span className="u">tok/s</span></div>
    <div className="num">{kv}<span className="u">%</span></div>
  </div>
);

const FLEET = [
  { id: "qwen2.5-7b-a",    gpu: "gpu-0", status: "healthy",  slots: ["busy","busy","busy","warm",""], tps: "142",  kv: "68" },
  { id: "qwen2.5-7b-b",    gpu: "gpu-0", status: "healthy",  slots: ["busy","busy","warm","",""],     tps: "128",  kv: "54" },
  { id: "llama-3.1-70b",   gpu: "gpu-1", status: "healthy",  slots: ["busy","busy","busy","busy","busy","busy","busy",""], tps: "84",   kv: "71" },
  { id: "mistral-small",   gpu: "gpu-0", status: "degraded", slots: ["busy","busy","",""],            tps: "31",   kv: "92" },
  { id: "deepseek-coder",  gpu: "gpu-2", status: "pending",  slots: ["","","",""],                     tps: "—",    kv: "—"  },
  { id: "phi-3.5-mini",    gpu: "gpu-2", status: "healthy",  slots: ["busy","warm","",""],            tps: "204",  kv: "23" },
];

const Dashboard = () => {
  const tpsSpark = [120, 140, 132, 158, 142, 168, 174, 162, 180, 192, 188, 210, 198, 220, 232, 218];
  const queueSpark = [3,5,4,6,8,10,12,9,11,14,12,15,13,12,10,12];
  const memSpark = [68, 70, 72, 71, 74, 78, 80, 82, 81, 84, 83, 85, 84, 86, 87, 86];
  const errSpark = [0,0,1,0,0,0,2,1,0,0,0,1,0,0,0,0];
  return (
    <div className="Page">
      <div className="PageInner">
        <div className="PageHead">
          <div>
            <h1 className="title">Dashboard</h1>
            <div className="sub">
              <span>Fleet overview · 6 running · 1 degraded · 0 failed</span>
              <span className="mono">refreshed 2s ago</span>
            </div>
          </div>
          <div className="actions">
            <span className="SearchHint">
              <Icon name="search" size={14} />
              <span className="ph">Search servers, requests…</span>
              <kbd>⌘K</kbd>
            </span>
            <button className="Btn secondary sm">
              <Icon name="refresh" size={14} /> Refresh
            </button>
            <button className="Btn primary sm">
              <Icon name="plus" size={14} /> Add server
            </button>
          </div>
        </div>

        <div className="grid grid-4" style={{ marginBottom: 16 }}>
          <MetricCard
            title="Throughput"
            value="1,184" unit="tok/s"
            delta="↑ 12.4% vs 1h"
            deltaKind="up"
            icon="activity"
            spark={tpsSpark}
          />
          <MetricCard
            title="Queue depth"
            value="12" unit="reqs"
            delta="↑ 3 since 5m"
            deltaKind="warn"
            icon="list"
            spark={queueSpark}
            sparkColor="var(--status-degraded-solid)"
          />
          <MetricCard
            title="GPU memory"
            value="84.2" unit="GB / 96 GB"
            delta="3 GPUs · avg 87%"
            deltaKind="warn"
            icon="cpu"
            spark={memSpark}
            sparkColor="var(--status-degraded-solid)"
          />
          <MetricCard
            title="Errors (5m)"
            value="4" unit="req"
            delta="↓ 0.34% rate"
            deltaKind="up"
            icon="zap"
            spark={errSpark}
            sparkColor="var(--status-failed-solid)"
          />
        </div>

        <div className="Card" style={{ marginBottom: 16 }}>
          <div className="hdr">
            <h3>Fleet</h3>
            <span className="meta">7 servers · gpu-0 · gpu-1 · gpu-2</span>
          </div>
          <div className="body" role="table" aria-label="Servers">
            <div className="FleetRow" role="row" style={{
              height: 32, color: "var(--text-tertiary)",
              font: "var(--weight-semibold) 10px var(--font-sans)",
              letterSpacing: "var(--tracking-wide)", textTransform: "uppercase",
              borderBottom: "1px solid var(--border-default)"
            }}>
              <div>Server</div>
              <div>Status</div>
              <div>Slots</div>
              <div style={{ textAlign: "right" }}>TPS</div>
              <div style={{ textAlign: "right" }}>KV cache</div>
            </div>
            {FLEET.map((s) => <FleetRow key={s.id} {...s} />)}
          </div>
        </div>
      </div>
    </div>
  );
};

// ─── AppShell — composition root ─────────────────────────────────────
const AppShell = ({ initialCollapsed = false }) => {
  const [collapsed, setCollapsed] = React.useState(initialCollapsed);
  const [active, setActive] = React.useState("dashboard");

  // Auto-collapse on initialCollapsed change (for reseating into resized artboards)
  React.useEffect(() => { setCollapsed(initialCollapsed); }, [initialCollapsed]);

  return (
    <div className={`AppShell ${collapsed ? "collapsed" : ""}`}>
      <Sidebar
        activeId={active}
        onNav={setActive}
        collapsed={collapsed}
        onToggle={() => setCollapsed(c => !c)}
      />
      <div className="Main">
        <TopBar
          crumbs={[{ label: NAV_ITEMS.find(n => n.id === active)?.label || "Dashboard" }]}
          conn="connected"
          connMeta="50051"
        />
        {active === "dashboard" ? <Dashboard /> : <EmptyPage label={NAV_ITEMS.find(n => n.id === active)?.label} />}
      </div>
    </div>
  );
};

const EmptyPage = ({ label }) => (
  <div className="Page">
    <div className="PageInner">
      <div className="PageHead">
        <div>
          <h1 className="title">{label}</h1>
          <div className="sub"><span className="mono">stub · navigate back to Dashboard</span></div>
        </div>
      </div>
      <div className="Card"><div className="body" style={{ padding: 48, textAlign: "center", color: "var(--text-tertiary)" }}>
        This view is a stub for the app-shell exploration. Open Dashboard to see populated content.
      </div></div>
    </div>
  </div>
);

Object.assign(window, { AppShell, Sidebar, TopBar, Dashboard, ConnBadge, Breadcrumb, Banner, Icon });

})();

/* ===== components.jsx ===== */
;(function(){
// Shared library components for state screens + the isolation sheet.

// ─── Spinner ─────────────────────────────────────────────────────────
const Spinner = ({ lg }) => <span className={`Spin ${lg ? "lg" : ""}`} role="status" aria-label="Loading" />;

// ─── Generic notification banner ─────────────────────────────────────
const ErrorBanner = ({ kind = "error", children, onRetry, retryLabel = "Retry", onDismiss, spinning }) => {
  const icon = kind === "error" ? "alertTriangle" : kind === "warning" ? "alertTriangle" : "info";
  return (
    <div className={`NBanner ${kind}`} role={kind === "info" ? "status" : "alert"}>
      {spinning ? <span className="Spin nspin" /> : <Icon name={icon} size={16} className="bi" />}
      <span className="grow">{children}</span>
      <span className="nact">
        {onRetry && <button className="BtnInline" onClick={onRetry}><Icon name="refresh" size={13} /> {retryLabel}</button>}
        {onDismiss && <button className="ndismiss" aria-label="Dismiss" onClick={onDismiss}><Icon name="x" size={14} /></button>}
      </span>
    </div>
  );
};

// ─── Generic empty state ─────────────────────────────────────────────
const CubeQuestion = () => (
  <svg viewBox="0 0 24 24" aria-hidden="true">
    <path d="M21 16V8a2 2 0 0 0-1-1.73l-7-4a2 2 0 0 0-2 0l-7 4A2 2 0 0 0 3 8v8a2 2 0 0 0 1 1.73l7 4a2 2 0 0 0 2 0l7-4A2 2 0 0 0 21 16z" />
    <path d="M10.6 10.2a1.6 1.6 0 0 1 3 .5c0 1.1-1.5 1.4-1.5 2.4" />
    <line x1="12.05" y1="16" x2="12.06" y2="16" />
  </svg>
);

const EmptyBlock = ({ icon, custom, good, title, desc, actionLabel, onAction, docLabel, docHref }) => (
  <div className="EmptyBlock" role="status">
    <div className={`eic ${good ? "good" : ""}`}>{custom || <Icon name={icon} size={28} />}</div>
    <h2>{title}</h2>
    <p>{desc}</p>
    {actionLabel && (
      <div className="eact"><button className="Btn primary" onClick={onAction}>{icon === "settings" ? null : null}{actionLabel}</button></div>
    )}
    {docLabel && (
      <a className="doclink" href={docHref || "#"}><Icon name="bookOpen" size={13} /> {docLabel}</a>
    )}
  </div>
);

// ─── Skeleton table rows ─────────────────────────────────────────────
const SkeletonRows = ({ cols = [120, 90, 110, 80, 70, 60, 70, 60], rows = 5 }) => (
  <table className="SkTable" aria-busy="true" aria-hidden="true">
    <tbody>
      {Array.from({ length: rows }).map((_, r) => (
        <tr key={r}>
          {cols.map((w, c) => (
            <td key={c}><span className="SkBar" style={{ width: w, animationDelay: `${(r * 0.08 + c * 0.03).toFixed(2)}s` }} /></td>
          ))}
        </tr>
      ))}
    </tbody>
  </table>
);

// ─── Filter chips ────────────────────────────────────────────────────
const FilterChips = ({ chips }) => (
  <div className="Chips">
    {chips.map((c, i) => (
      <span className="FChip" key={i}><b>{c.k}</b> {c.v}<span className="x" role="button" aria-label={`Remove ${c.k}`}><Icon name="x" size={11} /></span></span>
    ))}
  </div>
);

// ─── Model card ──────────────────────────────────────────────────────
const ModelCard = ({ m }) => (
  <div className="ModelCard">
    <div className="mc-head">
      <div style={{ minWidth: 0 }}>
        <div className="mc-id">{m.name}</div>
        <div style={{ marginTop: 6 }}><StatusBadge status={m.loaded ? "healthy" : "cancelled"} label={m.loaded ? "Loaded" : "Available"} /></div>
      </div>
      <div className="mc-icon"><Icon name="box" size={17} /></div>
    </div>
    <div className="mc-meta">
      <div className="m"><span className="k">Params</span><span className="v">{m.params}</span></div>
      <div className="m"><span className="k">Quant</span><span className="v">{m.quant}</span></div>
      <div className="m"><span className="k">Context</span><span className="v">{m.ctx}</span></div>
      <div className="m"><span className="k">File size</span><span className="v">{m.size}</span></div>
    </div>
    <div className="mc-foot">
      <span>{m.loaded ? `${m.servers} server${m.servers === 1 ? "" : "s"}` : "not in use"}</span>
      <span>{m.format}</span>
    </div>
  </div>
);

const SkModelCard = () => (
  <div className="ModelCard" aria-busy="true" aria-hidden="true">
    <div className="mc-head">
      <div style={{ flex: 1 }}>
        <span className="SkBar" style={{ width: "70%", height: 14 }} />
        <span className="SkBar" style={{ width: 64, height: 18, marginTop: 10, borderRadius: 4 }} />
      </div>
      <span className="SkBar" style={{ width: 32, height: 32, borderRadius: 6 }} />
    </div>
    <div className="mc-meta">
      {[0, 1, 2, 3].map(i => (
        <div className="m" key={i}>
          <span className="SkBar" style={{ width: 40, height: 8 }} />
          <span className="SkBar" style={{ width: 60, height: 12, marginTop: 4 }} />
        </div>
      ))}
    </div>
    <div className="mc-foot"><span className="SkBar" style={{ width: 80, height: 10 }} /></div>
  </div>
);

// ─── Toast ───────────────────────────────────────────────────────────
const TOAST_ICON = { success: "checkCircle", error: "alertTriangle", warning: "alertTriangle", info: "info" };
const Toast = ({ kind = "info", title, msg, onClose }) => (
  <div className={`Toast ${kind}`} role={kind === "error" ? "alert" : "status"}>
    <span className="ti"><Icon name={TOAST_ICON[kind]} size={18} /></span>
    <div className="tbody">
      <div className="tt">{title}</div>
      {msg && <div className="tm">{msg}</div>}
    </div>
    <button className="tx" aria-label="Dismiss" onClick={onClose}><Icon name="x" size={13} /></button>
  </div>
);

// ─── Chat bubbles ────────────────────────────────────────────────────
const ChatBubble = ({ variant }) => {
  if (variant === "user") {
    return <div className="Msg user"><div className="Bubble">Review this async scheduler for lock-ordering bugs under contention.</div></div>;
  }
  if (variant === "completed") {
    return (
      <div className="Msg assistant">
        <div className="Bubble">The acquire_slot path takes the slots write lock before reading the cache, while evict_cache does the reverse — a classic lock-ordering inversion that can deadlock under load. Compute the affinity score before taking the write lock.</div>
        <div className="meta"><span className="role">assistant</span><span>142 tok/s</span><span>·</span><span>TTFT 120ms</span><span>·</span><span>84 tokens</span></div>
      </div>
    );
  }
  if (variant === "streaming") {
    return (
      <div className="Msg assistant">
        <div className="Bubble">Looking at the decode loop now — the slot release on early-return looks correct, but the prefill batch sizing<span className="cursor" /></div>
        <div className="mrow">
          <span className="meta"><span className="role">assistant</span><span>generating…</span></span>
          <button className="Btn secondary sm"><Icon name="square" size={12} /> Cancel</button>
        </div>
      </div>
    );
  }
  return (
    <div className="Msg assistant error">
      <div className="Bubble">Generation failed: server llama-70b-gpu0 returned 503 (slot timeout after 30s).</div>
      <div className="mrow">
        <span className="meta"><span className="role">assistant</span><span>error</span></span>
        <button className="Btn secondary sm"><Icon name="refresh" size={12} /> Retry</button>
      </div>
    </div>
  );
};

// ─── DiffPreview (TOML) ──────────────────────────────────────────────
const DiffPreview = ({ rows }) => (
  <div className="Diff">
    {rows.map((r, i) => {
      const changed = r.old !== r.new;
      return (
        <div key={i} className={`DiffRow ${changed ? "changed" : "unchanged"}`}>
          <span className="key">{r.key}</span>
          <span className="vals">
            {changed ? (
              <><span className="old">{r.old}</span><span className="arrow">→</span><span className="new">{r.new}</span></>
            ) : (
              <span className="new">{r.new} <span style={{ color: "var(--text-tertiary)" }}>· unchanged</span></span>
            )}
          </span>
        </div>
      );
    })}
  </div>
);

// ─── Detailed registry card (Model Registry page) ───────────────────
const RegistryCard = ({ m, onOpenServer }) => {
  const dead = m.status === "failed";
  return (
    <div className={`RegCard is-${m.status}`}>
      <div className="rc-head">
        <div style={{ minWidth: 0 }}>
          <div className="rc-name">{m.name}</div>
          <div className="rc-quant"><span className="q">{m.quant}</span><span>·</span><span>{m.ctx} ctx</span></div>
        </div>
        <StatusBadge status={m.status} />
      </div>
      <div className="rc-meta">
        <div className="m"><span className="k">Throughput</span><span className={`v ${m.tpsWarn ? "warn" : ""} ${m.tps == null ? "muted" : ""}`}>{m.tps == null ? "—" : `${m.tps} tok/s`}</span></div>
        <div className="m"><span className="k">Latency P50</span><span className={`v ${m.p50Warn ? "warn" : ""} ${m.p50 == null ? "muted" : ""}`}>{m.p50 == null ? "—" : `${m.p50} ms`}</span></div>
        <div className="m"><span className="k">VRAM</span><span className={`v ${m.vram == null ? "muted" : ""}`}>{m.vram == null ? "—" : `${m.vram} GB`}</span></div>
        <div className="m"><span className="k">Context</span><span className="v">{m.ctx}</span></div>
      </div>
      <div className="rc-foot">
        <button className={`rc-server ${dead ? "dead" : ""}`} onClick={() => !dead && onOpenServer && onOpenServer(m.server)} disabled={dead}>
          <Icon name="server" size={12} /> {m.server}
        </button>
        <span style={{ font: "11px var(--font-mono)", color: "var(--text-tertiary)" }}>{dead ? "no recent data" : `${m.tps} tok/s avg`}</span>
      </div>
    </div>
  );
};

const RegistryGrid = ({ models, onOpenServer }) => (
  <div className="RegGrid">{models.map(m => <RegistryCard key={m.name} m={m} onOpenServer={onOpenServer} />)}</div>
);

Object.assign(window, {
  Spinner, ErrorBanner, EmptyBlock, SkeletonRows, FilterChips, ModelCard, SkModelCard,
  Toast, ChatBubble, DiffPreview, CubeQuestion, RegistryCard, RegistryGrid,
});

})();

/* ===== a11y-anno.jsx ===== */
;(function(){
// Shared annotation helpers — compute marker positions from the live DOM.
const { useState: useStateA, useRef: useRefA, useEffect: useEffectA, useLayoutEffect: useLayoutEffectA } = React;

// Positions overlay markers by querying selectors inside the host after layout.
function useAnno(hostRef, specs) {
  const [marks, setMarks] = useStateA([]);
  useEffectA(() => {
    const host = hostRef.current; if (!host) return;
    const compute = () => {
      const hb = host.getBoundingClientRect();
      const out = [];
      specs.forEach((sp) => {
        const el = typeof sp.sel === "function" ? sp.sel(host) : host.querySelector(sp.sel);
        if (!el) return;
        const r = el.getBoundingClientRect();
        out.push({ ...sp, x: r.left - hb.left, y: r.top - hb.top, w: r.width, h: r.height });
      });
      setMarks(out);
    };
    const t = setTimeout(compute, 120);
    const ro = new ResizeObserver(compute); ro.observe(host);
    return () => { clearTimeout(t); ro.disconnect(); };
  }, []);
  return marks;
}

// Landmark outline box
const Landmark = ({ kind, x, y, w, h, label, inset = 0 }) => (
  <div className={`Landmark ${kind}`} style={{ left: x + inset, top: y + inset, width: w - inset * 2, height: h - inset * 2 }}>
    <span className="ltag">{label}</span>
  </div>
);

// Numbered tab-order marker (centered on a corner of the target)
const TabMark = ({ x, y, w, h, n, anchor = "tl", alt }) => {
  const pos = {
    tl: { left: x + 12, top: y + 14 },
    tr: { left: x + w - 12, top: y + 14 },
    cl: { left: x + 14, top: y + h / 2 },
    c:  { left: x + w / 2, top: y + h / 2 },
  }[anchor];
  return <span className={`TabMark ${alt ? "alt" : ""}`} style={pos}>{n}</span>;
};

// aria-live ping label
const LiveMark = ({ x, y, w, h, text, anchor = "c" }) => {
  const pos = anchor === "tr" ? { left: x + w - 10, top: y + 12 } : { left: x + w / 2, top: y + h / 2 };
  return <span className="LiveMark" style={pos}><span className="pulse" />{text}</span>;
};

Object.assign(window, { useAnno, Landmark, TabMark, LiveMark });

})();
