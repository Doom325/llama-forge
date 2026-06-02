// LlamaForge — pages bundle: per-screen page components + their states.

/* ===== dashboard-page.jsx ===== */
;(function(){
// DashboardPage — primary operator screen. Lives inside AppShell's Main.
// Reuses Icon / StatusBadge / Sparkline from atoms.jsx.

const { useRef, useState, useEffect } = React;

// ─── Screen-reader audio summary ─────────────────────────────────────
const SUMMARY_TEXT =
  "Dashboard summary. Total throughput 142 tokens per second. Queue depth 7 of 100. " +
  "3 of 4 servers healthy. Average P50 latency 45 milliseconds, P99 210 milliseconds. " +
  "One server degraded: codellama-gpu2, throughput down to 12 tokens per second, latency elevated to 180 milliseconds. " +
  "One server failed: mistral-gpu3, CUDA out of memory.";

const SummaryBtn = () => {
  const [speaking, setSpeaking] = useState(false);
  const onClick = () => {
    const synth = window.speechSynthesis;
    if (!synth) return;
    if (speaking) { synth.cancel(); setSpeaking(false); return; }
    const u = new SpeechSynthesisUtterance(SUMMARY_TEXT);
    u.rate = 1.05; u.onend = () => setSpeaking(false);
    synth.cancel(); synth.speak(u); setSpeaking(true);
  };
  return (
    <button
      className={`SummaryBtn ${speaking ? "speaking" : ""}`}
      onClick={onClick}
      aria-label="Play an audio summary of the dashboard"
    >
      <Icon name={speaking ? "volume2" : "volume"} size={14} />
      {speaking ? "Stop summary" : "Dashboard summary"}
    </button>
  );
};

// ─── Radial gauge (queue depth) ──────────────────────────────────────
const Gauge = ({ value, max, cap }) => {
  const R = 22, C = 2 * Math.PI * R;
  const pct = Math.min(value / max, 1);
  const off = C * (1 - pct * 0.75); // 270° sweep
  const danger = pct > 0.85;
  return (
    <div className="Gauge">
      <svg width="58" height="58" viewBox="0 0 58 58" aria-hidden="true">
        <g transform="rotate(135 29 29)">
          <circle className="gtrack" cx="29" cy="29" r={R} fill="none" strokeWidth="6"
            strokeDasharray={`${C * 0.75} ${C}`} strokeLinecap="round"/>
          <circle className="gfill" cx="29" cy="29" r={R} fill="none" strokeWidth="6"
            strokeDasharray={C} strokeDashoffset={off} strokeLinecap="round"
            style={{ stroke: danger ? "var(--status-degraded-solid)" : "var(--accent-blue)" }}/>
        </g>
      </svg>
      <div className="glabel">
        <span className="pct">{Math.round(pct * 100)}%</span>
        <span className="cap">{cap}</span>
      </div>
    </div>
  );
};

// ─── Metric cards row ────────────────────────────────────────────────
const tput = [128,134,131,140,138,145,142,150,147,141,138,144,142,148,145,142];

const MetricsRow = () => (
  <div className="grid grid-4" style={{ marginBottom: 16 }}>
    {/* 1 · Throughput */}
    <div className="MetricCard">
      <div className="title"><span>Total throughput</span><Icon name="activity" size={14} /></div>
      <div className="value">142<span className="u">tok/s</span></div>
      <div className="delta up">↑ 8.4% · vs 5m ago</div>
      <div className="spark"><Sparkline points={tput} color="var(--status-healthy-solid)" /></div>
    </div>

    {/* 2 · Queue depth — gauge */}
    <div className="MetricCard">
      <div className="title"><span>Queue depth</span><Icon name="list" size={14} /></div>
      <div className="valrow">
        <div className="value">7<span className="u">/ 100</span></div>
      </div>
      <Gauge value={7} max={100} cap="capacity" />
    </div>

    {/* 3 · Active servers — capacity bar */}
    <div className="MetricCard">
      <div className="title"><span>Active servers</span><Icon name="server" size={14} /></div>
      <div className="valrow">
        <div className="value">3<span className="u">/ 4</span></div>
        <div className="sub2"><b>1</b> down</div>
      </div>
      <div className="CapBar" aria-hidden="true">
        <i className="on"/><i className="on"/><i className="on"/><i className="off"/>
      </div>
    </div>

    {/* 4 · Avg latency P50 + P99 */}
    <div className="MetricCard">
      <div className="title"><span>Avg latency · P50</span><Icon name="gauge" size={14} /></div>
      <div className="value">45<span className="u">ms</span></div>
      <div className="sub2" style={{ marginTop: 2 }}>P99: <b>210 ms</b></div>
      <div className="spark"><Sparkline points={[40,42,41,44,43,46,45,48,44,45,47,45,46,44,45,45]} color="var(--accent-blue)" /></div>
    </div>
  </div>
);

// ─── Server card ─────────────────────────────────────────────────────
const SlotBar = ({ total, busy, warm = 0, amber = false }) => (
  <div className={`SlotBar ${amber ? "amber" : ""}`} aria-label={`${busy} of ${total} slots in use`}>
    {Array.from({ length: total }).map((_, i) => (
      <i key={i} className={i < busy ? "busy" : i < busy + warm ? "warm" : ""} />
    ))}
  </div>
);

const ActionBtn = ({ children, variant = "secondary", onClick }) => (
  <button className={`Btn ${variant} sm`} onClick={(e) => { e.stopPropagation(); onClick && onClick(); }}>
    {children}
  </button>
);

const ServerCard = ({ s, onOpen, selected }) => {
  const open = () => onOpen && onOpen(s.id);
  const stopped = s.status === "stopped";
  return (
    <div
      className={`ServerCard is-${s.status} ${selected ? "selected" : ""}`}
      role="link" tabIndex={0}
      aria-current={selected ? "true" : undefined}
      aria-label={`${s.id}, ${s.status}. Open server detail.`}
      onClick={open}
      onKeyDown={(e) => { if (e.key === "Enter" || e.key === " ") { e.preventDefault(); open(); } }}
    >
      <div className="sc-head">
        <div style={{ minWidth: 0 }}>
          <div className="sc-id">{s.id}</div>
        </div>
        <StatusBadge status={stopped ? "cancelled" : s.status} label={stopped ? "Stopped" : undefined} />
      </div>

      <div className="sc-model">
        <Icon name="box" size={13} />
        <span style={{ overflow: "hidden", textOverflow: "ellipsis" }}>{s.model}</span>
      </div>

      {s.status === "failed" ? (
        <div className="sc-error">
          <svg className="icon" viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="1.75"
               strokeLinecap="round" strokeLinejoin="round" aria-hidden="true">
            <path d="M10.29 3.86 1.82 18a2 2 0 0 0 1.71 3h16.94a2 2 0 0 0 1.71-3L13.71 3.86a2 2 0 0 0-3.42 0z"/>
            <line x1="12" y1="9" x2="12" y2="13"/><line x1="12" y1="17" x2="12.01" y2="17"/>
          </svg>
          {s.error}
        </div>
      ) : stopped ? (
        <div className="sc-stats"><div className="stat full"><span className="k">Slots · 0/{s.total || 4} in use</span>
          <span style={{ font: "12px var(--font-mono)", color: "var(--text-tertiary)" }}>Process stopped · last exit 0</span></div></div>
      ) : (
        <div className="sc-stats">
          <div className="stat full">
            <span className="k">Slots · {s.busy}/{s.total} in use</span>
            <SlotBar total={s.total} busy={s.busy} warm={s.warm} amber={s.status === "degraded"} />
          </div>
          <div className="stat">
            <span className="k">Throughput</span>
            <span className={`v ${s.tpsWarn ? "warn" : ""}`}>{s.tps}<span className="u">tok/s</span></span>
          </div>
          {s.p50 != null && (
            <div className="stat">
              <span className="k">Latency P50</span>
              <span className={`v ${s.p50Warn ? "warn" : ""}`}>{s.p50}<span className="u">ms</span></span>
            </div>
          )}
        </div>
      )}

      <div className="sc-actions">
        {s.status === "failed" || stopped ? (
          <ActionBtn variant="primary">Start</ActionBtn>
        ) : (
          <>
            <ActionBtn variant="secondary">Restart</ActionBtn>
            <ActionBtn variant="secondary">Stop</ActionBtn>
          </>
        )}
      </div>
    </div>
  );
};

const DASH_SERVERS = [
  { id: "llama-70b-gpu0", status: "healthy",  model: "Llama 3.1 70B · Q4_K_M", total: 4, busy: 3, warm: 0, tps: "42", p50: "38" },
  { id: "llama-8b-gpu1",  status: "healthy",  model: "Llama 3.1 8B · Q8_0",    total: 8, busy: 1, warm: 0, tps: "85", p50: null },
  { id: "codellama-gpu2", status: "degraded", model: "CodeLlama 34B · Q5_K_M", total: 4, busy: 4, warm: 0, tps: "12", tpsWarn: true, p50: "180", p50Warn: true },
  { id: "mistral-gpu3",   status: "failed",   model: "Mistral 7B · Q5_K_M",    error: "CUDA out of memory" },
];

const ServerGrid = ({ onOpen }) => (
  <div className="ServerGrid">
    {DASH_SERVERS.map((s) => <ServerCard key={s.id} s={s} onOpen={onOpen} />)}
  </div>
);

// ─── ECharts time-series ─────────────────────────────────────────────
const axisLabels = Array.from({ length: 13 }, (_, i) => `-${60 - i * 5}m`);
const wob = (base, amp, n, seed) =>
  Array.from({ length: n }, (_, i) =>
    +(base + Math.sin(i * 0.5 + seed) * amp + Math.cos(i * 0.27 + seed) * amp * 0.5).toFixed(1));

const themeColors = (dark) => ({
  axis: dark ? "#94a3b8" : "#64748b",
  split: dark ? "#243154" : "#e2e8f0",
  text: dark ? "#cbd5e1" : "#475569",
  tipBg: dark ? "#1a2540" : "#ffffff",
  tipBorder: dark ? "#344166" : "#cbd5e1",
});

const baseGrid = { left: 44, right: 16, top: 12, bottom: 28 };

const lineOption = (dark, series, names) => {
  const c = themeColors(dark);
  return {
    grid: baseGrid,
    color: series.map((s) => s.color),
    tooltip: {
      trigger: "axis",
      backgroundColor: c.tipBg, borderColor: c.tipBorder, borderWidth: 1,
      textStyle: { color: c.text, fontFamily: "monospace", fontSize: 11 },
      axisPointer: { lineStyle: { color: c.axis } },
    },
    xAxis: {
      type: "category", data: axisLabels, boundaryGap: false,
      axisLine: { lineStyle: { color: c.split } },
      axisTick: { show: false },
      axisLabel: { color: c.axis, fontFamily: "monospace", fontSize: 10, interval: 1 },
    },
    yAxis: {
      type: "value",
      splitLine: { lineStyle: { color: c.split } },
      axisLabel: { color: c.axis, fontFamily: "monospace", fontSize: 10 },
    },
    series: series.map((s, i) => ({
      name: names[i], type: "line", data: s.data, smooth: true, symbol: "none",
      lineStyle: { width: 1.5 },
      emphasis: { focus: "series" },
    })),
  };
};

const ChartCard = ({ title, meta, legend, option, theme }) => {
  const ref = useRef(null);
  const [ready, setReady] = useState(false);
  useEffect(() => {
    let chart;
    const t = setTimeout(() => {
      if (!ref.current || !window.echarts) return;
      chart = window.echarts.init(ref.current, null, { renderer: "canvas" });
      chart.setOption(option(theme === "dark"));
      requestAnimationFrame(() => chart.resize());
      setReady(true);
    }, 850);
    const onR = () => chart && chart.resize();
    window.addEventListener("resize", onR);
    return () => { clearTimeout(t); window.removeEventListener("resize", onR); chart && chart.dispose(); };
  }, [theme]);
  return (
    <div className="Card ChartCard">
      <div className="hdr">
        <h3>{title} <span className="meta" style={{ marginLeft: 4 }}>{meta}</span></h3>
        <div className="legend">
          {legend.map((l) => (
            <span key={l.label}><i style={{ background: l.color }}/>{l.label}</span>
          ))}
        </div>
      </div>
      <div className="body" style={{ padding: 12 }}>
        <div className="ChartBox">
          <div ref={ref} className="ChartCanvas" style={{ opacity: ready ? 1 : 0 }} />
          {!ready && (
            <div className="ChartSkeleton" role="status" aria-label="Loading chart">
              <div className="grid" />
              <div className="spin" />
              <div className="lbl">Loading chart…</div>
            </div>
          )}
        </div>
      </div>
    </div>
  );
};

const TPUT_SERIES = [
  { color: "#16a34a", data: wob(42, 6, 13, 0.2) },
  { color: "#2563eb", data: wob(85, 10, 13, 1.1) },
  { color: "#7c3aed", data: wob(120, 14, 13, 2.3) },
  { color: "#d97706", data: wob(18, 8, 13, 3.0) },
];
const TPUT_NAMES = ["llama-70b-gpu0", "llama-8b-gpu1", "codellama-gpu2", "mistral-gpu3"];
const TPUT_LEGEND = [
  { label: "llama-70b", color: "#16a34a" },
  { label: "llama-8b",  color: "#2563eb" },
  { label: "codellama", color: "#7c3aed" },
  { label: "mistral",   color: "#d97706" },
];

const LAT_SERIES = [
  { color: "#2563eb", data: wob(45, 5, 13, 0.4) },
  { color: "#d97706", data: wob(95, 12, 13, 1.6) },
  { color: "#dc2626", data: wob(210, 28, 13, 2.7) },
];
const LAT_NAMES = ["P50", "P90", "P99"];
const LAT_LEGEND = [
  { label: "P50", color: "#2563eb" },
  { label: "P90", color: "#d97706" },
  { label: "P99", color: "#dc2626" },
];

const ChartsRow = ({ theme }) => (
  <div className="ChartsRow">
    <ChartCard
      title="Throughput" meta="· last 1 hour · tok/s"
      legend={TPUT_LEGEND} theme={theme}
      option={(dark) => lineOption(dark, TPUT_SERIES, TPUT_NAMES)}
    />
    <ChartCard
      title="Latency percentiles" meta="· last 1 hour · ms"
      legend={LAT_LEGEND} theme={theme}
      option={(dark) => lineOption(dark, LAT_SERIES, LAT_NAMES)}
    />
  </div>
);

// ─── Page composition ────────────────────────────────────────────────
const DashHead = ({ subtitle, scrape = "last scrape 2s ago" }) => (
  <div className="DashHead">
    <div className="titlewrap">
      <div className="titlerow">
        <h1>Dashboard</h1>
        <SummaryBtn />
      </div>
      <div className="sub">
        {subtitle || (
          <>
            <span>Fleet overview</span>
            <span className="sep">·</span>
            <span>3 healthy</span>
            <span className="sep">·</span>
            <span style={{ color: "var(--status-degraded-fg)" }}>1 degraded</span>
            <span className="sep">·</span>
            <span style={{ color: "var(--status-failed-fg)" }}>1 failed</span>
          </>
        )}
        <span className="sep">·</span>
        <span className="mono">{scrape}</span>
      </div>
    </div>
    <div className="actions">
      <button className="Btn secondary sm"><Icon name="refresh" size={14} /> Refresh all</button>
      <button className="Btn primary sm"><Icon name="plus" size={14} /> Add server</button>
    </div>
  </div>
);

const DashboardPage = ({ theme, onOpenServer }) => (
  <div className="Page">
    <div className="PageInner">
      <DashHead />

      <MetricsRow />
      <ServerGrid onOpen={onOpenServer} />
      <ChartsRow theme={theme} />
    </div>
  </div>
);

Object.assign(window, { DashboardPage, SummaryBtn, DashHead, MetricsRow, ServerGrid, ServerCard, ChartsRow, ChartCard, Gauge, SlotBar });

})();

/* ===== dashboard-skeleton.jsx ===== */
;(function(){
// SkeletonDashboard — what renders in the first ~300ms before API data.
// Sidebar + header come from the static shell (already loaded); only the
// content region is skeletonised. Every skeleton container is aria-busy.

const SkMetric = () => (
  <div className="SkMetric" aria-busy="true" aria-hidden="true">
    <span className="sk sk-title" />
    <span className="sk sk-value" />
    <span className="sk sk-delta" />
    <span className="sk sk-spark" />
  </div>
);

const SkServer = ({ rows = 2 }) => (
  <div className="SkServer" aria-busy="true" aria-hidden="true">
    <div className="sk-row">
      <span className="sk sk-id" />
      <span className="sk sk-badge" />
    </div>
    <span className="sk sk-model" />
    <span className="sk sk-bar" />
    <div className="sk-stats">
      <span className="sk sk-stat" />
      <span className="sk sk-stat" />
    </div>
    <div className="sk-actions">
      <span className="sk sk-btn" />
      <span className="sk sk-btn" />
    </div>
  </div>
);

const SkChart = () => (
  <div className="Card ChartCard">
    <div className="hdr">
      <h3><span className="sk" style={{ display: "inline-block", height: 13, width: 140, verticalAlign: "middle" }} /></h3>
      <span className="sk" style={{ height: 11, width: 180, borderRadius: 3 }} />
    </div>
    <div className="body" style={{ padding: 12 }}>
      <div className="SkChart" role="status" aria-busy="true" aria-label="Loading chart">
        <div className="sk-axes" />
        <span className="sk-loading">Loading…</span>
      </div>
    </div>
  </div>
);

const SkeletonDashboard = () => (
  <div className="Page">
    <div className="PageInner">
      {/* Page head is part of the static shell — title + summary affordance stay */}
      <div className="DashHead">
        <div className="titlewrap">
          <div className="titlerow">
            <h1>Dashboard</h1>
            <SummaryBtn />
          </div>
          <div className="sub" aria-busy="true">
            <span className="sk" style={{ height: 12, width: 260, borderRadius: 3 }} />
          </div>
        </div>
        <div className="actions">
          <button className="Btn secondary sm"><Icon name="refresh" size={14} /> Refresh all</button>
          <button className="Btn primary sm"><Icon name="plus" size={14} /> Add server</button>
        </div>
      </div>

      {/* Metric cards */}
      <div className="grid grid-4" style={{ marginBottom: 16 }} aria-busy="true" aria-label="Loading metrics">
        <SkMetric /><SkMetric /><SkMetric /><SkMetric />
      </div>

      {/* Server card grid — same auto-fill geometry, min-height 180 */}
      <div className="ServerGrid" aria-busy="true" aria-label="Loading servers">
        <SkServer /><SkServer /><SkServer /><SkServer />
      </div>

      {/* Charts */}
      <div className="ChartsRow">
        <SkChart />
        <SkChart />
      </div>
    </div>
  </div>
);

Object.assign(window, { SkeletonDashboard });

})();

/* ===== dashboard-states.jsx ===== */
;(function(){
// Dashboard empty + unreachable states.
// Reuses DashHead / MetricsRow pieces and Icon / StatusBadge from prior files.

// ─── Empty metric row (no data) ──────────────────────────────────────
const EmptyMetric = ({ title, icon, value, unit, note }) => (
  <div className="MetricCard">
    <div className="title"><span>{title}</span><Icon name={icon} size={14} /></div>
    <div className="value" style={{ color: "var(--text-tertiary)" }}>
      {value}{unit && <span className="u">{unit}</span>}
    </div>
    {note && <div className="sub2" style={{ marginTop: 2 }}>{note}</div>}
    <div className="spark" />
  </div>
);

const EmptyMetricsRow = () => (
  <div className="grid grid-4" style={{ marginBottom: 16 }}>
    <EmptyMetric title="Total throughput" icon="activity" value="—" note="no servers" />
    <EmptyMetric title="Queue depth"      icon="list"     value="0" unit="/ 100" />
    <EmptyMetric title="Active servers"   icon="server"   value="0" unit="/ 0" />
    <EmptyMetric title="Avg latency · P50" icon="gauge"    value="—" note="P99: —" />
  </div>
);

// ─── Empty state — no servers configured ─────────────────────────────
const ServerPlusGlyph = () => (
  <svg viewBox="0 0 24 24" aria-hidden="true">
    <rect x="2" y="3" width="14" height="7" rx="2"/>
    <rect x="2" y="14" width="14" height="7" rx="2"/>
    <line x1="6" y1="6.5" x2="6.01" y2="6.5"/>
    <line x1="6" y1="17.5" x2="6.01" y2="17.5"/>
    <line x1="20" y1="5.5" x2="20" y2="11.5"/>
    <line x1="17" y1="8.5" x2="23" y2="8.5"/>
  </svg>
);

const EmptyServers = ({ onConfigure }) => (
  <div className="EmptyState" role="region" aria-label="No servers configured">
    <div className="ic"><ServerPlusGlyph /></div>
    <h2>No servers configured</h2>
    <p>Add a server in Configuration to get started.</p>
    <div className="act">
      <button className="Btn primary" onClick={onConfigure}>
        <Icon name="settings" size={14} /> Go to Configuration
      </button>
    </div>
  </div>
);

// ─── Chart "no data" ─────────────────────────────────────────────────
const ChartNoData = ({ title, meta }) => (
  <div className="Card ChartCard">
    <div className="hdr">
      <h3>{title} <span className="meta" style={{ marginLeft: 4 }}>{meta}</span></h3>
    </div>
    <div className="body" style={{ padding: 12 }}>
      <div className="ChartNoData" role="img" aria-label={`${title}: no data`}>
        <div className="axes" />
        <span className="msg"><Icon name="activity" size={15} /> No data</span>
      </div>
    </div>
  </div>
);

const ChartsNoDataRow = () => (
  <div className="ChartsRow">
    <ChartNoData title="Throughput" meta="· last 1 hour · tok/s" />
    <ChartNoData title="Latency percentiles" meta="· last 1 hour · ms" />
  </div>
);

// ─── Page: empty (no servers) ────────────────────────────────────────
const DashEmpty = ({ onConfigure }) => (
  <div className="Page">
    <div className="PageInner">
      <DashHead
        subtitle={<><span>Fleet overview</span><span className="sep">·</span><span>0 servers</span></>}
        scrape="never"
      />
      <EmptyMetricsRow />
      <EmptyServers onConfigure={onConfigure} />
      <ChartsNoDataRow />
    </div>
  </div>
);

// ─── Backend error banner ────────────────────────────────────────────
const BackendBanner = ({ onRetry }) => (
  <div className="Banner error" role="alert">
    <Icon name="alertTriangle" size={16} className="bi" />
    <span className="btext">Cannot reach LlamaForge backend. Check if the service is running.</span>
    <button className="baction push" onClick={onRetry}>
      <Icon name="refresh" size={13} style={{ marginRight: 5, verticalAlign: "-2px" }} /> Retry
    </button>
  </div>
);

// ─── Page: unreachable, stale data ───────────────────────────────────
const DashStale = ({ theme, onRetry }) => (
  <div className="Page">
    <div className="PageInner">
      <div className="StaleStrip" role="status">
        <Icon name="clock" size={14} />
        <span>Showing last-known data — backend unreachable.</span>
        <span className="age">Last updated 45s ago</span>
      </div>
      <div className="is-stale" aria-hidden="true">
        <DashHead />
        <MetricsRow />
        <ServerGrid onOpen={() => {}} />
        <ChartsRow theme={theme} />
      </div>
    </div>
  </div>
);

// ─── Page: unreachable, never loaded (full-page error) ───────────────
const DashErrorFull = ({ onRetry }) => (
  <div className="Page">
    <div className="FullError" role="alert">
      <div className="ic"><Icon name="serverOff" size={30} /></div>
      <h2>Cannot reach LlamaForge backend</h2>
      <p>The orchestrator isn’t responding. Check that the service is running and that the gRPC port is reachable.</p>
      <span className="meta">grpc://127.0.0.1:50051 · connection refused</span>
      <div className="act">
        <button className="Btn primary" onClick={onRetry}><Icon name="refresh" size={14} /> Retry</button>
        <button className="Btn secondary"><Icon name="terminal" size={14} /> View logs</button>
      </div>
    </div>
  </div>
);

Object.assign(window, { DashEmpty, DashStale, DashErrorFull, BackendBanner });

})();

/* ===== server-detail.jsx ===== */
;(function(){
// Server Detail page — drill-down for a single server.
// states: "healthy" (also the base for the confirm modal), "restarting", "failed".
// Reuses Icon / StatusBadge / ChartCard from atoms.jsx + dashboard-page.jsx.

const { useRef: useRefSD, useEffect: useEffectSD } = React;

// ─── Action bar ──────────────────────────────────────────────────────
const ActionBar = ({ state }) => {
  if (state === "restarting") {
    return (
      <div className="ActionBar">
        <button className="Btn secondary" disabled>
          <span className="btn-spin" /> Restarting…
        </button>
        <button className="Btn secondary" disabled><Icon name="square" size={14} /> Stop</button>
        <button className="Btn danger-outline" disabled><Icon name="xOctagon" size={14} /> Force kill</button>
      </div>
    );
  }
  if (state === "failed") {
    return (
      <div className="ActionBar">
        <button className="Btn primary"><Icon name="power" size={14} /> Start</button>
        <button className="Btn secondary" disabled><Icon name="square" size={14} /> Stop</button>
        <button className="Btn danger-outline"><Icon name="xOctagon" size={14} /> Force kill</button>
      </div>
    );
  }
  if (state === "stopped") {
    return (
      <div className="ActionBar">
        <button className="Btn primary"><Icon name="power" size={14} /> Start</button>
      </div>
    );
  }
  // healthy
  return (
    <div className="ActionBar">
      <button className="Btn secondary"><Icon name="refresh" size={14} /> Restart</button>
      <button className="Btn secondary"><Icon name="square" size={14} /> Stop</button>
      <button className="Btn danger-outline"><Icon name="xOctagon" size={14} /> Force kill</button>
    </div>
  );
};

const STATUS_FOR = { healthy: "healthy", restarting: "pending", failed: "failed" };

// ─── Slot grid ───────────────────────────────────────────────────────
const LIVE_SLOTS = [
  { n: 0, state: "active", task: "Generating", user: "user-42", tokens: "340 tokens", age: "12s" },
  { n: 1, state: "active", task: "Prefilling", user: "user-17", tokens: "128 tokens", age: null },
  { n: 2, state: "active", task: "Generating", user: "user-8",  tokens: "890 tokens", age: "28s" },
  { n: 3, state: "idle" },
];

const SlotCell = ({ slot, blank }) => {
  if (blank) {
    return (
      <div className="SlotCell offline">
        <div className="s-head">
          <span className="s-name">slot-{slot.n}</span>
          <span className="s-pill"><span className="dot" />Offline</span>
        </div>
        <div className="s-dash">—</div>
      </div>
    );
  }
  if (slot.state === "idle") {
    return (
      <div className="SlotCell idle">
        <div className="s-head">
          <span className="s-name">slot-{slot.n}</span>
          <span className="s-pill"><span className="dot" />Idle</span>
        </div>
        <div className="s-task" style={{ color: "var(--text-tertiary)" }}>Available</div>
      </div>
    );
  }
  return (
    <div className="SlotCell active">
      <div className="s-head">
        <span className="s-name">slot-{slot.n}</span>
        <span className="s-pill"><span className="dot" />Active</span>
      </div>
      <div className="s-task">{slot.task} · {slot.user}</div>
      <div className="s-meta">
        <span>{slot.tokens}</span>
        {slot.age && <><span className="u">·</span><span>{slot.age}</span></>}
      </div>
    </div>
  );
};

const SlotGrid = ({ state }) => {
  const blank = state === "restarting" || state === "failed";
  return (
    <div className="Section">
      <div className="SectionHead">
        <h2>Slots</h2>
        <span className="meta">{blank ? "0/4 in use" : "3/4 in use"}</span>
      </div>
      <div className="SlotGrid">
        {LIVE_SLOTS.map((s) => <SlotCell key={s.n} slot={s} blank={blank} />)}
      </div>
    </div>
  );
};

// ─── Metrics column ──────────────────────────────────────────────────
const Bar = ({ pct, kind }) => (
  <div className={`Bar ${kind || ""}`}><i style={{ width: `${Math.min(pct, 100)}%` }} /></div>
);

const MetricsList = ({ state }) => {
  const live = state === "healthy";
  const m = (v) => live ? v : "—";
  return (
    <div className="Card">
      <div className="MetricList">
        <div className="row"><span className="k">Throughput</span>
          <span className={`v ${live ? "" : "muted"}`}>{m("42")}{live && <span style={{ color: "var(--text-tertiary)" }}> tok/s</span>}</span></div>
        <div className="row"><span className="k">Latency</span>
          {live ? (
            <span className="lat"><span><b>P50</b>38ms</span><span><b>P90</b>95ms</span><span><b>P99</b>210ms</span></span>
          ) : <span className="v muted">—</span>}
        </div>
        <div className="row"><span className="k">Error rate</span>
          <span className={`v ${live ? "" : "muted"}`}>{m("0.2")}{live && "%"}</span></div>
        <div className="barrow">
          <div className="top"><span className="k">VRAM usage</span>
            <span className={`v ${live ? "" : "muted"}`}>{live ? "18.2 / 24.0 GB" : "—"}</span></div>
          <Bar pct={live ? 76 : 0} />
        </div>
        <div className="barrow">
          <div className="top"><span className="k">KV cache utilization</span>
            <span className={`v ${live ? "" : "muted"}`}>{live ? "72%" : "—"}</span></div>
          <Bar pct={live ? 72 : 0} />
        </div>
        <div className="row" style={{ borderBottom: "none" }}><span className="k">Uptime</span>
          <span className={`v ${live ? "" : "muted"}`}>{live ? "4d 12h 30m" : "—"}</span></div>
      </div>
    </div>
  );
};

// ─── Detail chart (throughput + latency overlay) ─────────────────────
const sdTheme = (dark) => ({
  axis: dark ? "#94a3b8" : "#64748b",
  split: dark ? "#243154" : "#e2e8f0",
  text: dark ? "#cbd5e1" : "#475569",
  tipBg: dark ? "#1a2540" : "#ffffff",
  tipBorder: dark ? "#344166" : "#cbd5e1",
});
const sdLabels = Array.from({ length: 13 }, (_, i) => `-${60 - i * 5}m`);
const sdWob = (base, amp, seed) =>
  Array.from({ length: 13 }, (_, i) =>
    +(base + Math.sin(i * 0.5 + seed) * amp + Math.cos(i * 0.27 + seed) * amp * 0.5).toFixed(1));

const detailChartOption = (dark) => {
  const c = sdTheme(dark);
  return {
    grid: { left: 44, right: 48, top: 12, bottom: 28 },
    color: ["#2563eb", "#d97706"],
    tooltip: {
      trigger: "axis", backgroundColor: c.tipBg, borderColor: c.tipBorder, borderWidth: 1,
      textStyle: { color: c.text, fontFamily: "monospace", fontSize: 11 },
      axisPointer: { lineStyle: { color: c.axis } },
    },
    xAxis: {
      type: "category", data: sdLabels, boundaryGap: false,
      axisLine: { lineStyle: { color: c.split } }, axisTick: { show: false },
      axisLabel: { color: c.axis, fontFamily: "monospace", fontSize: 10, interval: 1 },
    },
    yAxis: [
      { type: "value", name: "tok/s", nameTextStyle: { color: c.axis, fontFamily: "monospace", fontSize: 10, align: "left" },
        splitLine: { lineStyle: { color: c.split } },
        axisLabel: { color: c.axis, fontFamily: "monospace", fontSize: 10 } },
      { type: "value", name: "ms", position: "right", nameTextStyle: { color: c.axis, fontFamily: "monospace", fontSize: 10, align: "right" },
        splitLine: { show: false },
        axisLabel: { color: c.axis, fontFamily: "monospace", fontSize: 10 } },
    ],
    series: [
      { name: "Throughput", type: "line", yAxisIndex: 0, data: sdWob(42, 6, 0.3), smooth: true, symbol: "none", lineStyle: { width: 1.5 } },
      { name: "P50 latency", type: "line", yAxisIndex: 1, data: sdWob(40, 9, 1.4), smooth: true, symbol: "none", lineStyle: { width: 1.5 } },
    ],
  };
};

const DetailChart = ({ theme }) => (
  <ChartCard
    title="Throughput + latency" meta="· this server · last 1 hour"
    theme={theme}
    legend={[{ label: "tok/s", color: "#2563eb" }, { label: "P50 ms", color: "#d97706" }]}
    option={detailChartOption}
  />
);

// ─── Config summary ──────────────────────────────────────────────────
const CONFIG = [
  ["Model path", "/models/llama-3.1-70b-instruct.Q4_K_M.gguf"],
  ["GPU layers", "81"],
  ["Context size", "32768"],
  ["Batch size", "512"],
  ["Threads", "16"],
  ["Flash attention", "on"],
];
const ConfigSummary = () => (
  <div className="Section">
    <div className="SectionHead">
      <h2>Configuration</h2>
      <button className="Btn secondary sm"><Icon name="edit" size={13} /> Edit configuration</button>
    </div>
    <div className="Card">
      <dl className="ConfigKV">
        {CONFIG.map(([k, v]) => (
          <React.Fragment key={k}>
            <dt>{k}</dt><dd>{v}</dd>
          </React.Fragment>
        ))}
      </dl>
    </div>
  </div>
);

// ─── Confirm dialog (force kill) ─────────────────────────────────────
const ConfirmDialog = ({ onCancel, onConfirm }) => {
  const ref = useRefSD(null);
  useEffectSD(() => {
    const node = ref.current;
    if (!node) return;
    const focusables = () => [...node.querySelectorAll("button")];
    const init = node.querySelector("[data-autofocus]") || focusables()[0];
    init && init.focus({ preventScroll: true });
    const onKey = (e) => {
      if (e.key === "Escape") { e.preventDefault(); onCancel && onCancel(); return; }
      if (e.key === "Tab") {
        const f = focusables();
        const first = f[0], last = f[f.length - 1];
        if (e.shiftKey && document.activeElement === first) { e.preventDefault(); last.focus(); }
        else if (!e.shiftKey && document.activeElement === last) { e.preventDefault(); first.focus(); }
      }
    };
    node.addEventListener("keydown", onKey);
    return () => node.removeEventListener("keydown", onKey);
  }, []);
  return (
    <div className="ModalBackdrop">
      <div className="Modal" role="dialog" aria-modal="true" aria-labelledby="fk-title" aria-describedby="fk-msg" ref={ref}>
        <div className="m-top">
          <div className="m-icon"><Icon name="xOctagon" size={18} /></div>
          <div>
            <h2 id="fk-title">Force kill server</h2>
            <p id="fk-msg">
              This will immediately terminate the server process. All active requests
              (<b>3</b>) will be lost. This cannot be undone.
            </p>
          </div>
        </div>
        <div className="m-actions">
          <button className="Btn secondary" data-autofocus onClick={onCancel}>Cancel</button>
          <button className="Btn danger" onClick={onConfirm}><Icon name="xOctagon" size={14} /> Force kill</button>
        </div>
      </div>
    </div>
  );
};

// ─── Page composition ────────────────────────────────────────────────
const ServerDetailPage = ({ state = "healthy", theme }) => (
  <div className="Page">
    <div className="PageInner">
      <div className="DetailHead">
        <div className="titlerow">
          <h1>llama-70b-gpu0</h1>
          <StatusBadge status={STATUS_FOR[state]} />
          <span className="spacer" />
          <ActionBar state={state} />
        </div>
        {state === "failed" && (
          <div className="DetailBanner" role="alert">
            <Icon name="alertTriangle" size={16} className="bi" />
            <span>Restart failed: <code>CUDA out of memory</code>. Reduce GPU layers and retry.</span>
          </div>
        )}
        <div className="endpoint">grpc://127.0.0.1:50051 · endpoint 127.0.0.1:8081 · gpu-0</div>
      </div>

      <SlotGrid state={state} />

      <div className="Section">
        <div className="SectionHead"><h2>Live metrics</h2>
          <span className="meta">{state === "healthy" ? "scrape 2s ago" : "no live data"}</span></div>
        <div className="DetailCols">
          <MetricsList state={state} />
          <DetailChart theme={theme} />
        </div>
      </div>

      <ConfigSummary />
    </div>
  </div>
);

Object.assign(window, { ServerDetailPage, ConfirmDialog, ActionBar });

})();

/* ===== queue-page.jsx ===== */
;(function(){
// Queue Inspector — sortable/filterable table of queued requests, with
// cancel-confirm and an empty state. Self-contained (its own confirm modal).

const { useState: useStateQ, useRef: useRefQ, useEffect: useEffectQ } = React;

const QUEUE_ROWS = [
  { pos: 1, id: "req-a8f2", model: "llama-70b",     prio: "high",   prioVal: 900, requestor: "user-42",       status: "processing", etaS: null },
  { pos: 2, id: "req-b3c1", model: "llama-70b",     prio: "normal", prioVal: 500, requestor: "user-17",       status: "queued",     etaS: 4 },
  { pos: 3, id: "req-c7d4", model: "codellama-34b", prio: "normal", prioVal: 500, requestor: "api-svc",       status: "queued",     etaS: 8 },
  { pos: 4, id: "req-d9e6", model: "llama-8b",      prio: "low",    prioVal: 100, requestor: "user-8",        status: "queued",     etaS: 12 },
  { pos: 5, id: "req-e1f8", model: "mistral-7b",    prio: "low",    prioVal: 100, requestor: "chat-gateway",  status: "queued",     etaS: 16 },
  { pos: 6, id: "req-f2a3", model: "llama-8b",      prio: "low",    prioVal: 100, requestor: "batch-svc",     status: "queued",     etaS: 20 },
  { pos: 7, id: "req-1a90", model: "codellama-34b", prio: "low",    prioVal: 100, requestor: "nightly-batch", status: "queued",     etaS: 24 },
];

const PRIO_LABEL = { high: "High", normal: "Normal", low: "Low" };

// ─── Cancel-confirm modal (self-contained, focus-trapped) ────────────
const QConfirm = ({ reqId, onKeep, onConfirm }) => {
  const ref = useRefQ(null);
  useEffectQ(() => {
    const node = ref.current; if (!node) return;
    const f = () => [...node.querySelectorAll("button")];
    (node.querySelector("[data-autofocus]") || f()[0]).focus({ preventScroll: true });
    const onKey = (e) => {
      if (e.key === "Escape") { e.preventDefault(); onKeep && onKeep(); return; }
      if (e.key === "Tab") {
        const fs = f(), first = fs[0], last = fs[fs.length - 1];
        if (e.shiftKey && document.activeElement === first) { e.preventDefault(); last.focus(); }
        else if (!e.shiftKey && document.activeElement === last) { e.preventDefault(); first.focus(); }
      }
    };
    node.addEventListener("keydown", onKey);
    return () => node.removeEventListener("keydown", onKey);
  }, []);
  return (
    <div className="ModalBackdrop">
      <div className="Modal" role="dialog" aria-modal="true" aria-labelledby="qc-t" aria-describedby="qc-m" ref={ref}>
        <div className="m-top">
          <div className="m-icon"><Icon name="xOctagon" size={18} /></div>
          <div>
            <h2 id="qc-t">Cancel request</h2>
            <p id="qc-m">Cancel queued request <b>{reqId}</b>? It will be removed from the queue and the requestor will receive a cancellation. This cannot be undone.</p>
          </div>
        </div>
        <div className="m-actions">
          <button className="Btn secondary" data-autofocus onClick={onKeep}>Keep in queue</button>
          <button className="Btn danger" onClick={onConfirm}><Icon name="xOctagon" size={14} /> Cancel request</button>
        </div>
      </div>
    </div>
  );
};

// ─── Badges ──────────────────────────────────────────────────────────
const QBadge = ({ status }) =>
  status === "processing" ? (
    <span className="QBadge processing"><Icon name="refresh" size={12} className="ic" /> Processing</span>
  ) : (
    <span className="QBadge queued"><Icon name="clock" size={12} className="ic" /> Queued</span>
  );

const Prio = ({ prio, val }) => (
  <span className={`Prio ${prio}`}>
    <span className="dot" />
    <span className="lab">{PRIO_LABEL[prio]}</span>
    <span className="val">({val})</span>
  </span>
);

// ─── Sortable header cell ────────────────────────────────────────────
const Th = ({ label, col, sort, onSort, numeric, children }) => {
  const active = sort.key === col;
  const cls = ["sortable", numeric ? "num" : "", active ? "sorted" : ""].join(" ");
  return (
    <th className={cls} onClick={() => onSort(col)} aria-sort={active ? (sort.dir === "asc" ? "ascending" : "descending") : "none"}>
      <span className="th-in">
        {label}
        <svg className="arr" viewBox="0 0 24 24" aria-hidden="true">
          {active && sort.dir === "asc"
            ? <polyline points="18 15 12 9 6 15" />
            : <polyline points="6 9 12 15 18 9" />}
        </svg>
      </span>
    </th>
  );
};

// ─── Page ────────────────────────────────────────────────────────────
const QueuePage = ({ forceEmpty }) => {
  const [rows, setRows] = useStateQ(QUEUE_ROWS);
  const [statusF, setStatusF] = useStateQ("all");
  const [modelF, setModelF] = useStateQ("all");
  const [prioF, setPrioF] = useStateQ("all");
  const [sort, setSort] = useStateQ({ key: "pos", dir: "asc" });
  const [confirmId, setConfirmId] = useStateQ(null);
  const [removing, setRemoving] = useStateQ(null);

  const models = ["all", ...Array.from(new Set(QUEUE_ROWS.map(r => r.model)))];

  const onSort = (key) => setSort(s =>
    s.key === key ? { key, dir: s.dir === "asc" ? "desc" : "asc" } : { key, dir: "asc" });

  let view = rows.filter(r =>
    (statusF === "all" || r.status === statusF) &&
    (modelF === "all" || r.model === modelF) &&
    (prioF === "all" || r.prio === prioF)
  );
  view = [...view].sort((a, b) => {
    let av, bv;
    if (sort.key === "pos") { av = a.pos; bv = b.pos; }
    else if (sort.key === "prio") { av = a.prioVal; bv = b.prioVal; }
    else { av = a.etaS == null ? -1 : a.etaS; bv = b.etaS == null ? -1 : b.etaS; }
    return sort.dir === "asc" ? av - bv : bv - av;
  });

  const doCancel = (id) => {
    setConfirmId(null);
    setRemoving(id);
    setTimeout(() => { setRows(rs => rs.filter(r => r.id !== id)); setRemoving(null); }, 220);
  };

  const counts = {
    all: rows.length,
    queued: rows.filter(r => r.status === "queued").length,
    processing: rows.filter(r => r.status === "processing").length,
  };

  const isEmpty = forceEmpty || rows.length === 0;
  const filteredEmpty = !isEmpty && view.length === 0;

  return (
    <div className="Page">
      <div className="PageInner">
        <div className="QHead">
          <h1>Queue Inspector</h1>
          <div className="sub"><b>{isEmpty ? 0 : rows.length}</b> request{rows.length === 1 ? "" : "s"} queued</div>
        </div>

        {!isEmpty && (
          <div className="FilterBar">
            <div className="Toggle" role="group" aria-label="Filter by status">
              {["all", "queued", "processing"].map(s => (
                <button key={s} className={statusF === s ? "on" : ""} onClick={() => setStatusF(s)}
                  aria-pressed={statusF === s}>
                  {s === "all" ? "All" : s === "queued" ? "Queued" : "Processing"}
                  <span className="cnt">{counts[s]}</span>
                </button>
              ))}
            </div>
            <div className="Field">
              <label htmlFor="qf-model">Model</label>
              <span className="SelWrap">
                <select id="qf-model" value={modelF} onChange={e => setModelF(e.target.value)}>
                  {models.map(m => <option key={m} value={m}>{m === "all" ? "All models" : m}</option>)}
                </select>
                <svg className="chev" viewBox="0 0 24 24"><polyline points="6 9 12 15 18 9" /></svg>
              </span>
            </div>
            <div className="Field">
              <label htmlFor="qf-prio">Priority</label>
              <span className="SelWrap">
                <select id="qf-prio" value={prioF} onChange={e => setPrioF(e.target.value)}>
                  <option value="all">All priorities</option>
                  <option value="high">High</option>
                  <option value="normal">Normal</option>
                  <option value="low">Low</option>
                </select>
                <svg className="chev" viewBox="0 0 24 24"><polyline points="6 9 12 15 18 9" /></svg>
              </span>
            </div>
          </div>
        )}

        {isEmpty ? (
          <div className="QEmpty" role="status">
            <div className="ic"><Icon name="check" size={28} /></div>
            <h2>Queue is empty</h2>
            <p>All requests are being served immediately. New requests will appear here when they queue.</p>
          </div>
        ) : (
          <div className="QTableWrap">
            <table className="QTable">
              <thead>
                <tr>
                  <Th label="Position" col="pos" sort={sort} onSort={onSort} numeric />
                  <th>Request ID</th>
                  <th>Model</th>
                  <Th label="Priority" col="prio" sort={sort} onSort={onSort} />
                  <th>Requestor</th>
                  <th>Status</th>
                  <Th label="ETA" col="eta" sort={sort} onSort={onSort} numeric />
                  <th className="num">Actions</th>
                </tr>
              </thead>
              <tbody>
                {filteredEmpty ? (
                  <tr><td colSpan={8} style={{ height: 96, textAlign: "center", color: "var(--text-tertiary)", fontFamily: "var(--font-mono)", fontSize: 13 }}>
                    No requests match these filters.
                  </td></tr>
                ) : view.map(r => (
                  <tr key={r.id} className={`entering ${removing === r.id ? "removing" : ""}`}>
                    <td className="num pos">{r.pos}</td>
                    <td><span className="rid">{r.id}</span></td>
                    <td><span className="model">{r.model}</span></td>
                    <td><Prio prio={r.prio} val={r.prioVal} /></td>
                    <td><span className="requestor">{r.requestor}</span></td>
                    <td><QBadge status={r.status} /></td>
                    <td className={`num eta ${r.etaS == null ? "dash" : ""}`}>{r.etaS == null ? "—" : `~${r.etaS}s`}</td>
                    <td className="actions">
                      {r.status === "queued"
                        ? <button className="LinkBtn" onClick={() => setConfirmId(r.id)}>Cancel</button>
                        : <span className="dash">—</span>}
                    </td>
                  </tr>
                ))}
              </tbody>
            </table>
          </div>
        )}
      </div>

      {confirmId && (
        <QConfirm reqId={confirmId} onKeep={() => setConfirmId(null)} onConfirm={() => doCancel(confirmId)} />
      )}
    </div>
  );
};

Object.assign(window, { QueuePage, QConfirm });

})();

/* ===== explorer-page.jsx ===== */
;(function(){
// Request Explorer — dense historical browser with filters, sortable virtual
// table, and a right-hand detail drawer.

const { useState: useStateX, useMemo: useMemoX, useRef: useRefX, useEffect: useEffectX } = React;

// ─── Data ────────────────────────────────────────────────────────────
const X_MODELS = ["llama-70b", "llama-8b", "codellama-34b", "mistral-7b", "phi-3-mini"];
const X_TASKS = ["chat", "completion", "code_review", "summarization"];
const X_STATUS = [
  { key: "completed", badge: "healthy" },
  { key: "failed", badge: "failed" },
  { key: "cancelled", badge: "cancelled" },
  { key: "timeout", badge: "timeout" },
];
const X_REQUESTORS = ["user-42", "user-17", "user-8", "api-svc", "chat-gateway", "code-review-bot", "nightly-batch", "support-bot"];

const SEED_ROWS = [
  { ts: "10:42:15", id: "req-a8f2", model: "llama-70b", task: "chat", status: "completed", ttft: 120, tps: 42, lat: 1.2, tok: 340 },
  { ts: "10:41:58", id: "req-b3c1", model: "llama-8b", task: "completion", status: "completed", ttft: 45, tps: 85, lat: 0.8, tok: 680 },
  { ts: "10:41:30", id: "req-c7d4", model: "codellama-34b", task: "code_review", status: "failed", ttft: null, tps: null, lat: null, tok: null },
  { ts: "10:40:55", id: "req-d9e6", model: "llama-70b", task: "chat", status: "completed", ttft: 150, tps: 38, lat: 2.1, tok: 890 },
];

// Build ~60 rows deterministically for the virtual-scroll feel.
const genRows = () => {
  const rows = [...SEED_ROWS];
  let h = 10, m = 40, s = 30;
  for (let i = 0; i < 56; i++) {
    s -= 17 + (i % 5); while (s < 0) { s += 60; m -= 1; } while (m < 0) { m += 60; h -= 1; }
    const st = X_STATUS[(i * 7) % 4].key;
    const ok = st === "completed";
    const model = X_MODELS[(i * 3) % X_MODELS.length];
    const id = "req-" + (0x1000 + i * 7).toString(16);
    rows.push({
      ts: `${String(h).padStart(2, "0")}:${String(m).padStart(2, "0")}:${String(s).padStart(2, "0")}`,
      id, model, task: X_TASKS[(i * 5) % X_TASKS.length], status: st,
      ttft: ok ? 40 + ((i * 17) % 180) : (st === "timeout" ? null : null),
      tps: ok ? 30 + ((i * 13) % 200) : null,
      lat: ok ? +(0.6 + ((i * 7) % 30) / 10).toFixed(1) : (st === "timeout" ? 30.0 : null),
      tok: ok ? 80 + ((i * 53) % 1400) : null,
    });
  }
  return rows;
};
const ALL_ROWS = genRows();

// ─── Multi-select (model) ────────────────────────────────────────────
const MultiSelect = ({ options, selected, onChange }) => {
  const [open, setOpen] = useStateX(false);
  const ref = useRefX(null);
  useEffectX(() => {
    const onDoc = (e) => { if (ref.current && !ref.current.contains(e.target)) setOpen(false); };
    document.addEventListener("mousedown", onDoc);
    return () => document.removeEventListener("mousedown", onDoc);
  }, []);
  const label = selected.length === 0 ? "All models"
    : selected.length === options.length ? "All models"
    : `${selected.length} selected`;
  const toggle = (o) => onChange(selected.includes(o) ? selected.filter(x => x !== o) : [...selected, o]);
  return (
    <div className="MultiSel" ref={ref}>
      <button className="MultiSelBtn" onClick={() => setOpen(o => !o)} aria-expanded={open}>
        {label}
        <svg className="chev" viewBox="0 0 24 24"><polyline points="6 9 12 15 18 9" /></svg>
      </button>
      {open && (
        <div className="MultiPop" role="listbox" aria-multiselectable="true">
          {options.map(o => (
            <label key={o}>
              <input type="checkbox" checked={selected.includes(o)} onChange={() => toggle(o)} />
              {o}
            </label>
          ))}
        </div>
      )}
    </div>
  );
};

// ─── Sortable header ─────────────────────────────────────────────────
const XTh = ({ label, col, sort, onSort, numeric }) => {
  const active = sort.key === col;
  return (
    <th className={`sortable ${numeric ? "num" : ""} ${active ? "sorted" : ""}`}
        onClick={() => onSort(col)}
        aria-sort={active ? (sort.dir === "asc" ? "ascending" : "descending") : "none"}>
      <span className="th-in">{label}
        <svg className="arr" viewBox="0 0 24 24" aria-hidden="true">
          {active && sort.dir === "asc" ? <polyline points="18 15 12 9 6 15" /> : <polyline points="6 9 12 15 18 9" />}
        </svg>
      </span>
    </th>
  );
};

const fmt = (v, unit) => v == null ? <span className="dim">—</span> : <>{v}{unit}</>;

// ─── Detail drawer ───────────────────────────────────────────────────
const Preview = ({ text }) => {
  const [exp, setExp] = useStateX(false);
  return (
    <div>
      <div className={`Preview ${exp ? "expanded" : "collapsed"}`}>{text}</div>
      <button className="PrevToggle" onClick={() => setExp(e => !e)}>{exp ? "Show less" : "Show more"}</button>
    </div>
  );
};

const PROMPT = `You are a senior Rust engineer. Review the following async scheduler implementation for correctness under high contention. Focus on the slot-acquisition path and the KV-cache affinity heuristic.\n\nfn acquire_slot(&self, req: &Request) -> Option<SlotId> {\n    let mut slots = self.slots.write();\n    // ... 240 more lines\n}`;
const RESPONSE = `The slot-acquisition path has a potential lock-ordering inversion. acquire_slot takes self.slots.write() and then, inside the affinity check, calls self.cache.read() — but evict_cache() acquires those in the opposite order, which can deadlock under contention.\n\nRecommended fix: compute the affinity score before taking the slots write lock, or switch to a single combined lock guarding both maps. The decode loop otherwise looks correct.`;

const DetailDrawer = ({ row, open, onClose }) => {
  const ref = useRefX(null);
  useEffectX(() => {
    if (!open) return;
    const node = ref.current; if (!node) return;
    const onKey = (e) => { if (e.key === "Escape") onClose && onClose(); };
    node.addEventListener("keydown", onKey);
    const closeBtn = node.querySelector(".DClose");
    closeBtn && closeBtn.focus({ preventScroll: true });
    return () => node.removeEventListener("keydown", onKey);
  }, [open]);
  const r = row || SEED_ROWS[0];
  const badge = (X_STATUS.find(s => s.key === r.status) || {}).badge || "healthy";
  const statusLabel = r.status.charAt(0).toUpperCase() + r.status.slice(1);
  return (
    <>
      <div className={`DrawerScrim ${open ? "open" : ""}`} onClick={onClose} />
      <aside className={`Drawer ${open ? "open" : ""}`} role="dialog" aria-modal="false"
             aria-label={`Request ${r.id} detail`} ref={ref} aria-hidden={!open}>
        <div className="d-head">
          <div>
            <div className="eyebrow">Request detail</div>
            <div className="rid">{r.id}</div>
            <div className="sub">
              <StatusBadge status={badge} label={statusLabel} />
              <span>{r.ts}</span>
            </div>
          </div>
          <button className="DClose" onClick={onClose} aria-label="Close detail"><Icon name="x" size={15} /></button>
        </div>

        <div className="d-body">
          <div className="DSection Timing">
            <div className="dlabel">Timing breakdown · {r.lat != null ? `${r.lat}s total` : "—"}</div>
            <div className="bar">
              <i className="queue" style={{ width: "42%" }} />
              <i className="prefill" style={{ width: "10%" }} />
              <i className="decode" style={{ width: "48%" }} />
            </div>
            <div className="legend">
              <div className="leg"><span className="sw" style={{ background: "var(--text-tertiary)" }} /><span className="k">Queue wait</span><span className="v">0.50s</span></div>
              <div className="leg"><span className="sw" style={{ background: "var(--accent-blue-border)" }} /><span className="k">Prefill</span><span className="v">0.12s</span></div>
              <div className="leg"><span className="sw" style={{ background: "var(--accent-blue)" }} /><span className="k">Decode</span><span className="v">0.58s</span></div>
            </div>
          </div>

          <div className="DSection">
            <div className="dlabel">Placement</div>
            <dl className="DKV">
              <dt>Server</dt><dd>llama-70b-gpu0</dd>
              <dt>Slot</dt><dd>slot-2</dd>
              <dt>Model</dt><dd>Llama 3.1 70B · Q4_K_M</dd>
              <dt>Task</dt><dd>{r.task}</dd>
            </dl>
          </div>

          <div className="DSection">
            <div className="dlabel">Parameters</div>
            <div className="Params">
              <span className="Chip"><b>temp</b>0.7</span>
              <span className="Chip"><b>top_p</b>0.95</span>
              <span className="Chip"><b>max_tokens</b>2048</span>
            </div>
          </div>

          <div className="DSection">
            <div className="dlabel">Prompt · {r.tok != null ? `${r.tok} tokens out` : "—"}</div>
            <Preview text={PROMPT} />
          </div>

          <div className="DSection">
            <div className="dlabel">Response</div>
            <Preview text={RESPONSE} />
          </div>
        </div>
      </aside>
    </>
  );
};

// ─── Page ────────────────────────────────────────────────────────────
const ExplorerPage = ({ initialSelectedId }) => {
  const [range, setRange] = useStateX("1h");
  const [models, setModels] = useStateX([]);
  const [task, setTask] = useStateX("all");
  const [status, setStatus] = useStateX("all");
  const [requestor, setRequestor] = useStateX("");
  const [sort, setSort] = useStateX({ key: "ts", dir: "desc" });
  const [selected, setSelected] = useStateX(initialSelectedId || null);

  const onSort = (key) => setSort(s => s.key === key ? { key, dir: s.dir === "asc" ? "desc" : "asc" } : { key, dir: "desc" });

  const view = useMemoX(() => {
    let rs = [...ALL_ROWS];
    const dir = sort.dir === "asc" ? 1 : -1;
    rs.sort((a, b) => {
      const av = a[sort.key], bv = b[sort.key];
      if (av == null && bv == null) return 0;
      if (av == null) return 1; if (bv == null) return -1;
      if (typeof av === "string") return av < bv ? -dir : av > bv ? dir : 0;
      return (av - bv) * dir;
    });
    return rs;
  }, [sort]);

  const selRow = ALL_ROWS.find(r => r.id === selected);

  return (
    <div className="Page">
      <div className="PageInner" style={{ maxWidth: 1400 }}>
        <div className="DashHead" style={{ marginBottom: 16 }}>
          <div className="titlewrap">
            <div className="titlerow"><h1 style={{ font: "var(--weight-semibold) 22px var(--font-sans)", margin: 0 }}>Request Explorer</h1></div>
            <div className="sub"><span>Historical request browser</span></div>
          </div>
          <div className="actions">
            <button className="Btn secondary sm"><Icon name="download" size={14} /> Export CSV</button>
            <button className="Btn secondary sm"><Icon name="download" size={14} /> Export JSON</button>
          </div>
        </div>

        {/* Filter bar */}
        <div className="ExpFilters">
          <div className="FField">
            <label>Time range</label>
            <div className="Toggle" role="group" aria-label="Time range">
              {[["1h", "Last 1h"], ["24h", "Last 24h"], ["7d", "Last 7d"], ["custom", "Custom…"]].map(([k, l]) => (
                <button key={k} className={range === k ? "on" : ""} onClick={() => setRange(k)} aria-pressed={range === k}>{l}</button>
              ))}
            </div>
          </div>
          <div className="FField">
            <label>Model</label>
            <MultiSelect options={X_MODELS} selected={models} onChange={setModels} />
          </div>
          <div className="FField">
            <label>Task type</label>
            <span className="SelWrap">
              <select value={task} onChange={e => setTask(e.target.value)}>
                <option value="all">All tasks</option>
                {X_TASKS.map(t => <option key={t} value={t}>{t}</option>)}
              </select>
              <svg className="chev" viewBox="0 0 24 24"><polyline points="6 9 12 15 18 9" /></svg>
            </span>
          </div>
          <div className="FField">
            <label>Status</label>
            <span className="SelWrap">
              <select value={status} onChange={e => setStatus(e.target.value)}>
                <option value="all">All statuses</option>
                {X_STATUS.map(s => <option key={s.key} value={s.key}>{s.key}</option>)}
              </select>
              <svg className="chev" viewBox="0 0 24 24"><polyline points="6 9 12 15 18 9" /></svg>
            </span>
          </div>
          <div className="FField">
            <label htmlFor="x-req">Requestor</label>
            <input id="x-req" className="Input" list="x-requestors" placeholder="Search requestor…"
                   value={requestor} onChange={e => setRequestor(e.target.value)} />
            <datalist id="x-requestors">{X_REQUESTORS.map(r => <option key={r} value={r} />)}</datalist>
          </div>
          <div className="FActions">
            <button className="Btn primary"><Icon name="sliders" size={14} /> Apply filters</button>
            <button className="Btn secondary">Reset</button>
          </div>
        </div>

        {/* Results header */}
        <div className="ExpResults">
          <span className="count"><b>2,847</b> requests</span>
          <span className="vsep" />
          <span className="muted">Page 1 of 57</span>
          <div className="right">
            <span className="rpp">Rows per page
              <span className="SelWrap">
                <select defaultValue="50" style={{ height: 28 }}>
                  <option>25</option><option>50</option><option>100</option>
                </select>
                <svg className="chev" viewBox="0 0 24 24"><polyline points="6 9 12 15 18 9" /></svg>
              </span>
            </span>
          </div>
        </div>

        {/* Table */}
        <div className="ExpTableWrap">
          <div className="ExpScroll">
            <table className="ExpTable">
              <thead>
                <tr>
                  <XTh label="Timestamp" col="ts" sort={sort} onSort={onSort} />
                  <th>Request ID</th>
                  <th>Model</th>
                  <th>Task type</th>
                  <th>Status</th>
                  <XTh label="TTFT" col="ttft" sort={sort} onSort={onSort} numeric />
                  <XTh label="TPS" col="tps" sort={sort} onSort={onSort} numeric />
                  <XTh label="Latency" col="lat" sort={sort} onSort={onSort} numeric />
                  <XTh label="Tokens" col="tok" sort={sort} onSort={onSort} numeric />
                </tr>
              </thead>
              <tbody>
                {view.map(r => {
                  const badge = (X_STATUS.find(s => s.key === r.status) || {}).badge;
                  const lbl = r.status.charAt(0).toUpperCase() + r.status.slice(1);
                  return (
                    <tr key={r.id} className={selected === r.id ? "selected" : ""} onClick={() => setSelected(r.id)}>
                      <td className="ts">{r.ts}</td>
                      <td className="rid">{r.id}</td>
                      <td className="model">{r.model}</td>
                      <td className="task">{r.task}</td>
                      <td className="status-cell"><StatusBadge status={badge} label={lbl} /></td>
                      <td className="num">{fmt(r.ttft, "ms")}</td>
                      <td className="num">{fmt(r.tps, "")}</td>
                      <td className="num">{fmt(r.lat, "s")}</td>
                      <td className="num">{fmt(r.tok, "")}</td>
                    </tr>
                  );
                })}
              </tbody>
            </table>
          </div>
          <div className="ExpFoot">
            <span>Showing 50 of 2,847 · virtualized</span>
            <div className="pager">
              <button className="PagerBtn" disabled>‹</button>
              <button className="PagerBtn on">1</button>
              <button className="PagerBtn">2</button>
              <button className="PagerBtn">3</button>
              <span style={{ padding: "0 4px" }}>…</span>
              <button className="PagerBtn">57</button>
              <button className="PagerBtn">›</button>
            </div>
          </div>
        </div>
      </div>

      <DetailDrawer row={selRow} open={!!selected} onClose={() => setSelected(null)} />
    </div>
  );
};

Object.assign(window, { ExplorerPage });

})();

/* ===== config-page.jsx ===== */
;(function(){
// Configuration — server config editor with tabs, dirty tracking, diff modal,
// and a concurrent-edit banner.

const { useState: useStateC, useRef: useRefC, useEffect: useEffectC } = React;

const SERVERS_LIST = ["llama-70b-gpu0", "llama-8b-gpu1", "codellama-gpu2", "mistral-gpu3"];

const DEFAULTS = {
  modelPath: "/models/llama-3.1-70b-q4_k_m.gguf",
  gpuLayers: 35,
  contextSize: 4096,
  batchSize: 512,
  threads: 8,
  port: 8080,
  maxSlots: 4,
};

// snake_case labels for the diff view
const DIFF_KEYS = [
  ["model_path", "modelPath"],
  ["gpu_layers", "gpuLayers"],
  ["context_size", "contextSize"],
  ["batch_size", "batchSize"],
  ["threads", "threads"],
  ["max_slots", "maxSlots"],
];

// ─── Diff modal ──────────────────────────────────────────────────────
const DiffModal = ({ baseline, values, onCancel, onConfirm }) => {
  const ref = useRefC(null);
  useEffectC(() => {
    const node = ref.current; if (!node) return;
    const f = () => [...node.querySelectorAll("button")];
    (node.querySelector("[data-autofocus]") || f()[0]).focus({ preventScroll: true });
    const onKey = (e) => {
      if (e.key === "Escape") { e.preventDefault(); onCancel && onCancel(); return; }
      if (e.key === "Tab") {
        const fs = f(), first = fs[0], last = fs[fs.length - 1];
        if (e.shiftKey && document.activeElement === first) { e.preventDefault(); last.focus(); }
        else if (!e.shiftKey && document.activeElement === last) { e.preventDefault(); first.focus(); }
      }
    };
    node.addEventListener("keydown", onKey);
    return () => node.removeEventListener("keydown", onKey);
  }, []);
  const changed = DIFF_KEYS.filter(([, k]) => String(baseline[k]) !== String(values[k])).length;
  return (
    <div className="ModalBackdrop">
      <div className="Modal wide" role="dialog" aria-modal="true" aria-labelledby="diff-t" ref={ref}>
        <div className="diff-head">
          <div>
            <h2 id="diff-t">Review changes</h2>
            <p>{changed} field{changed === 1 ? "" : "s"} changed on <span style={{ fontFamily: "var(--font-mono)" }}>llama-70b-gpu0</span>. Confirm to write to <span style={{ fontFamily: "var(--font-mono)" }}>llamaforge.toml</span>.</p>
          </div>
        </div>
        <div className="Diff">
          {DIFF_KEYS.map(([label, k]) => {
            const isChanged = String(baseline[k]) !== String(values[k]);
            return (
              <div key={k} className={`DiffRow ${isChanged ? "changed" : "unchanged"}`}>
                <span className="key">{label}</span>
                <span className="vals">
                  {isChanged ? (
                    <>
                      <span className="old">{String(baseline[k])}</span>
                      <span className="arrow">→</span>
                      <span className="new">{String(values[k])}</span>
                    </>
                  ) : (
                    <span className="new">{String(values[k])} <span style={{ color: "var(--text-tertiary)" }}>· unchanged</span></span>
                  )}
                </span>
              </div>
            );
          })}
        </div>
        <div className="diff-summary">port is read-only and requires a restart to change · not included here.</div>
        <div className="m-actions">
          <button className="Btn secondary" onClick={onCancel}>Cancel</button>
          <button className="Btn primary" data-autofocus onClick={onConfirm}><Icon name="check" size={14} /> Confirm &amp; save</button>
        </div>
      </div>
    </div>
  );
};

// ─── Form field ──────────────────────────────────────────────────────
const Field = ({ id, label, full, changed, req, children, hint, hintChanged }) => (
  <div className={`FRow ${full ? "full" : ""}`}>
    <label htmlFor={id}>
      <span>{label}</span>
      {req && <span className="req"><Icon name="lock" size={11} className="lock" /> {req}</span>}
    </label>
    {children}
    {hint && <span className={`FHint ${hintChanged ? "changed" : ""}`}>{hint}</span>}
  </div>
);

// ─── Page ────────────────────────────────────────────────────────────
const ConfigPage = ({ initialValues, openModal, showBanner }) => {
  const [baseline, setBaseline] = useStateC({ ...DEFAULTS });
  const [values, setValues] = useStateC({ ...DEFAULTS, ...(initialValues || {}) });
  const [modal, setModal] = useStateC(!!openModal);
  const [tab, setTab] = useStateC("server");
  const bannerRef = useRefC(null);

  useEffectC(() => {
    if (showBanner && bannerRef.current) {
      const b = bannerRef.current.querySelector("button");
      b && b.focus({ preventScroll: true });
    }
  }, [showBanner]);

  const set = (k, v) => setValues(vs => ({ ...vs, [k]: v }));
  const dirty = JSON.stringify(values) !== JSON.stringify(baseline);
  const isCh = (k) => String(values[k]) !== String(baseline[k]);

  const reset = () => setValues({ ...baseline });
  const save = () => { setBaseline({ ...values }); setModal(false); };

  return (
    <div className="Page">
      <div className="PageInner" style={{ maxWidth: 1000 }}>
        <div className="CfgHead"><h1>Configuration</h1></div>

        <div className="Tabs" role="tablist">
          {[["server", "Server config"], ["priority", "Priority rules"], ["global", "Global settings"]].map(([k, l]) => (
            <button key={k} role="tab" aria-selected={tab === k} className={`Tab ${tab === k ? "active" : ""}`} onClick={() => setTab(k)}>{l}</button>
          ))}
        </div>

        {showBanner && (
          <div className="CfgBanner" role="alert" ref={bannerRef}>
            <Icon name="alertTriangle" size={16} className="bi" />
            <span>Configuration was updated by another source. Reload to see the latest changes.</span>
            <span className="reload"><button className="Btn sm">Reload</button></span>
          </div>
        )}

        <div className="FormCard">
          <div className="fc-head">
            <span className="lbl">Server:</span>
            <span className="SelWrap">
              <select defaultValue="llama-70b-gpu0">
                {SERVERS_LIST.map(s => <option key={s} value={s}>{s}</option>)}
              </select>
              <svg className="chev" viewBox="0 0 24 24"><polyline points="6 9 12 15 18 9" /></svg>
            </span>
            <span style={{ marginLeft: "auto", font: "11px var(--font-mono)", color: "var(--text-tertiary)" }}>gpu-0 · running</span>
          </div>

          <div className="FormGrid">
            <Field id="cfg-model" label="Model path" full changed={isCh("modelPath")}>
              <input id="cfg-model" className={`FInput ${isCh("modelPath") ? "changed" : ""}`}
                     value={values.modelPath} onChange={e => set("modelPath", e.target.value)} />
            </Field>

            <Field id="cfg-gpu" label="GPU layers" hint={`Range 0–100${isCh("gpuLayers") ? ` · was ${baseline.gpuLayers}` : ""}`} hintChanged={isCh("gpuLayers")}>
              <div className="NumRange">
                <input type="range" min="0" max="100" value={values.gpuLayers} onChange={e => set("gpuLayers", +e.target.value)} aria-label="GPU layers" />
                <input id="cfg-gpu" type="number" min="0" max="100" className={`FInput ${isCh("gpuLayers") ? "changed" : ""}`}
                       value={values.gpuLayers} onChange={e => set("gpuLayers", +e.target.value)} />
              </div>
            </Field>

            <Field id="cfg-ctx" label="Context size" hint={isCh("contextSize") ? `was ${baseline.contextSize}` : "tokens"} hintChanged={isCh("contextSize")}>
              <span className="SelWrap" style={{ width: "100%" }}>
                <select id="cfg-ctx" className={`FInput ${isCh("contextSize") ? "changed" : ""}`} style={{ width: "100%", fontFamily: "var(--font-mono)" }}
                        value={values.contextSize} onChange={e => set("contextSize", +e.target.value)}>
                  {[2048, 4096, 8192, 16384].map(o => <option key={o} value={o}>{o}</option>)}
                </select>
                <svg className="chev" viewBox="0 0 24 24"><polyline points="6 9 12 15 18 9" /></svg>
              </span>
            </Field>

            <Field id="cfg-batch" label="Batch size" hint={isCh("batchSize") ? `was ${baseline.batchSize}` : "tokens per batch"} hintChanged={isCh("batchSize")}>
              <input id="cfg-batch" type="number" className={`FInput ${isCh("batchSize") ? "changed" : ""}`}
                     value={values.batchSize} onChange={e => set("batchSize", +e.target.value)} />
            </Field>

            <Field id="cfg-threads" label="Threads" hint={isCh("threads") ? `was ${baseline.threads}` : "CPU threads"} hintChanged={isCh("threads")}>
              <input id="cfg-threads" type="number" className={`FInput ${isCh("threads") ? "changed" : ""}`}
                     value={values.threads} onChange={e => set("threads", +e.target.value)} />
            </Field>

            <Field id="cfg-port" label="Port" req="Restart required">
              <input id="cfg-port" type="number" className="FInput" value={values.port} disabled readOnly />
            </Field>

            <Field id="cfg-slots" label="Max concurrent slots" hint={isCh("maxSlots") ? `was ${baseline.maxSlots}` : "parallel requests"} hintChanged={isCh("maxSlots")}>
              <input id="cfg-slots" type="number" min="1" max="64" className={`FInput ${isCh("maxSlots") ? "changed" : ""}`}
                     value={values.maxSlots} onChange={e => set("maxSlots", +e.target.value)} />
            </Field>
          </div>

          <div className="FormFoot">
            <div className={`DirtyInd ${dirty ? "" : "hidden"}`}>
              <span className="dot" /> Unsaved changes
            </div>
            <div className="FootBtns">
              <button className="Btn secondary" onClick={reset} disabled={!dirty} style={{ borderColor: "transparent", background: "transparent" }}>Reset</button>
              <button className="Btn secondary" onClick={() => setModal(true)} disabled={!dirty}>Review changes</button>
              <button className="Btn primary" onClick={() => setModal(true)} disabled={!dirty}><Icon name="check" size={14} /> Save</button>
            </div>
          </div>
        </div>
      </div>

      {modal && (
        <DiffModal baseline={baseline} values={values} onCancel={() => setModal(false)} onConfirm={save} />
      )}
    </div>
  );
};

Object.assign(window, { ConfigPage });

})();

/* ===== priority-rules.jsx ===== */
;(function(){
// Priority Rules editor — table with inline row editing + weight validation.
// Self-contained Configuration page (Priority rules tab active).

const { useState: useStateP, useRef: useRefP, useEffect: useEffectP } = React;

const RULE_FIELDS = ["task_type", "requestor", "model"];
const SEED_RULES = [
  { id: 1, field: "task_type", value: "code_review", weight: 900 },
  { id: 2, field: "task_type", value: "chat", weight: 500 },
  { id: 3, field: "requestor", value: "api-svc-critical", weight: 800 },
  { id: 4, field: "task_type", value: "summarization", weight: 500 },
];

const tier = (w) => w >= 700 ? "t-high" : w >= 400 ? "t-norm" : "t-low";
const validateWeight = (w) => {
  if (w === "" || w == null || isNaN(+w)) return "Priority is required";
  if (+w < 0 || +w > 1000) return "Priority must be between 0 and 1000";
  return null;
};

const RuleRow = ({ rule, n, editing, draft, onEdit, onDelete, onChange, onSave, onCancel }) => {
  if (editing) {
    const err = validateWeight(draft.weight);
    return (
      <tr className="editing">
        <td className="rnum">{n}</td>
        <td>
          <span className="RSel">
            <select value={draft.field} onChange={e => onChange("field", e.target.value)} aria-label="Match field">
              {RULE_FIELDS.map(f => <option key={f} value={f}>{f}</option>)}
            </select>
            <svg className="chev" viewBox="0 0 24 24"><polyline points="6 9 12 15 18 9" /></svg>
          </span>
        </td>
        <td><input className="RInput" value={draft.value} onChange={e => onChange("value", e.target.value)} aria-label="Match value" placeholder="value" /></td>
        <td>
          <input className={`RInput num ${err ? "invalid" : ""}`} type="number" min="0" max="1000"
                 value={draft.weight} onChange={e => onChange("weight", e.target.value)} aria-label="Priority weight"
                 aria-invalid={!!err} />
          {err && <div className="FieldErr"><Icon name="alertTriangle" size={12} /> {err}</div>}
        </td>
        <td className="actions">
          <div className="RuleActions">
            <button className="Btn primary sm" onClick={onSave} disabled={!!err}><Icon name="check" size={13} /> Save</button>
            <button className="Btn secondary sm" onClick={onCancel}>Cancel</button>
          </div>
        </td>
      </tr>
    );
  }
  return (
    <tr>
      <td className="rnum">{n}</td>
      <td className="rfield">{rule.field}</td>
      <td className="rvalue">{rule.value}</td>
      <td>
        <div className="WCell">
          <span className="wv">{rule.weight}</span>
          <span className="WBar"><i className={tier(rule.weight)} style={{ width: `${rule.weight / 10}%` }} /></span>
        </div>
      </td>
      <td className="actions">
        <div className="RuleActions">
          <button className="Btn secondary sm" onClick={onEdit}><Icon name="edit" size={13} /> Edit</button>
          <button className="Btn danger-outline sm" onClick={onDelete}>Delete</button>
        </div>
      </td>
    </tr>
  );
};

const PriorityRules = ({ initialEdit, initialDraft }) => {
  const [rules, setRules] = useStateP(SEED_RULES);
  const [editId, setEditId] = useStateP(initialEdit != null ? SEED_RULES[initialEdit].id : null);
  const [draft, setDraft] = useStateP(
    initialEdit != null ? { ...SEED_RULES[initialEdit], ...(initialDraft || {}) } : null
  );

  const startEdit = (r) => { setEditId(r.id); setDraft({ ...r }); };
  const change = (k, v) => setDraft(d => ({ ...d, [k]: v }));
  const cancel = () => { setEditId(null); setDraft(null); };
  const save = () => {
    if (validateWeight(draft.weight)) return;
    setRules(rs => rs.map(r => r.id === editId ? { ...draft, weight: +draft.weight } : r).filter(r => r.value !== "" ? true : false));
    cancel();
  };
  const del = (id) => setRules(rs => rs.filter(r => r.id !== id));
  const add = () => {
    const id = Math.max(0, ...rules.map(r => r.id)) + 1;
    const nr = { id, field: "task_type", value: "", weight: 500 };
    setRules(rs => [...rs, nr]); setEditId(id); setDraft({ ...nr });
  };

  return (
    <div className="RulesCard">
      <table className="RulesTable">
        <thead>
          <tr>
            <th style={{ width: 40 }}>#</th>
            <th>Match field</th>
            <th>Match value</th>
            <th>Priority weight</th>
            <th className="actions">Actions</th>
          </tr>
        </thead>
        <tbody>
          {rules.map((r, i) => (
            <RuleRow key={r.id} rule={r} n={i + 1}
              editing={editId === r.id} draft={editId === r.id ? draft : null}
              onEdit={() => startEdit(r)} onDelete={() => del(r.id)}
              onChange={change} onSave={save} onCancel={cancel} />
          ))}
        </tbody>
      </table>
      <div className="RulesFoot">
        <button className="Btn secondary" onClick={add}><Icon name="plus" size={14} /> Add rule</button>
        <div className="RulesHint">Higher weight wins. Ties break by arrival order. Range 0–1000.</div>
      </div>
    </div>
  );
};

// Full Configuration shell with the Priority rules tab active.
const PriorityRulesPage = ({ initialEdit, initialDraft }) => {
  const [tab, setTab] = useStateP("priority");
  return (
    <div className="Page">
      <div className="PageInner" style={{ maxWidth: 1000 }}>
        <div className="CfgHead"><h1>Configuration</h1></div>
        <div className="Tabs" role="tablist">
          {[["server", "Server config"], ["priority", "Priority rules"], ["global", "Global settings"]].map(([k, l]) => (
            <button key={k} role="tab" aria-selected={tab === k} className={`Tab ${tab === k ? "active" : ""}`} onClick={() => setTab(k)}>{l}</button>
          ))}
        </div>
        {tab === "priority"
          ? <PriorityRules initialEdit={initialEdit} initialDraft={initialDraft} />
          : <div className="RulesCard"><div style={{ padding: 28, font: "13px var(--font-mono)", color: "var(--text-tertiary)" }}>Open the “{tab === "server" ? "Server config" : "Global settings"}” screen for this tab.</div></div>}
      </div>
    </div>
  );
};

Object.assign(window, { PriorityRulesPage, PriorityRules });

})();

/* ===== playground.jsx ===== */
;(function(){
// Playground — parameter sidebar + chat pane + input. Two states: ready / streaming.

const { useState: useStatePG } = React;

const PG_MODELS = [
  "Llama 3.1 70B · Q4_K_M",
  "Llama 3.1 8B · Q8_0",
  "CodeLlama 34B · Q5_K_M",
  "Mistral 7B · Q5_K_M",
];

// ─── Parameter sidebar ───────────────────────────────────────────────
const SliderRow = ({ name, min, max, step, value, onChange }) => (
  <div className="ParamRow">
    <div className="pr-top">
      <span className="pr-name">{name}</span>
      <input className="pr-num" type="number" min={min} max={max} step={step} value={value}
             onChange={e => onChange(+e.target.value)} aria-label={name} />
    </div>
    <input type="range" min={min} max={max} step={step} value={value}
           onChange={e => onChange(+e.target.value)} aria-label={`${name} slider`} />
  </div>
);

const DEFAULTS_PG = { temp: 0.7, topP: 0.95, maxTokens: 2048, repeat: 1.1 };

const ParamSidebar = ({ onCompare, compare }) => {
  const [p, setP] = useStatePG({ ...DEFAULTS_PG });
  const set = (k) => (v) => setP(s => ({ ...s, [k]: v }));
  return (
    <aside className="PGParams" aria-label="Generation parameters">
      <div className="pp-sec modelsel">
        <div className="pp-label">{compare ? "Model A" : "Model"}</div>
        <span className="SelWrap">
          <select defaultValue={PG_MODELS[0]}>{PG_MODELS.map(m => <option key={m}>{m}</option>)}</select>
          <svg className="chev" viewBox="0 0 24 24"><polyline points="6 9 12 15 18 9" /></svg>
        </span>
        {compare && (
          <div style={{ marginTop: 12 }}>
            <div className="pp-label">Model B</div>
            <span className="SelWrap">
              <select defaultValue={PG_MODELS[1]}>{PG_MODELS.map(m => <option key={m}>{m}</option>)}</select>
              <svg className="chev" viewBox="0 0 24 24"><polyline points="6 9 12 15 18 9" /></svg>
            </span>
          </div>
        )}
      </div>

      <div className="pp-sec">
        <div className="pp-label">Generation parameters</div>
        <SliderRow name="Temperature" min={0} max={2} step={0.05} value={p.temp} onChange={set("temp")} />
        <SliderRow name="Top P" min={0} max={1} step={0.01} value={p.topP} onChange={set("topP")} />
        <div className="ParamRow">
          <div className="pr-top"><span className="pr-name">Max tokens</span></div>
          <input className="pr-int" type="number" min={1} max={32768} value={p.maxTokens}
                 onChange={e => set("maxTokens")(+e.target.value)} aria-label="Max tokens" />
        </div>
        <SliderRow name="Repeat penalty" min={0} max={2} step={0.05} value={p.repeat} onChange={set("repeat")} />
      </div>

      <div className="pp-foot">
        <button className="Btn secondary" onClick={() => setP({ ...DEFAULTS_PG })}
                style={{ borderColor: "transparent", background: "transparent", color: "var(--text-secondary)" }}>
          Reset to defaults
        </button>
        {!compare && (
          <button className="Btn secondary" onClick={onCompare}><Icon name="plus" size={14} /> Compare with another model</button>
        )}
      </div>
    </aside>
  );
};

// ─── Messages ────────────────────────────────────────────────────────
const UserMsg = ({ children }) => (
  <div className="Msg user"><div className="Bubble">{children}</div></div>
);

const AssistantMsg = ({ children, metrics, streaming, onCancel }) => (
  <div className="Msg assistant">
    <div className="Bubble">{children}{streaming && <span className="cursor" />}</div>
    <div className="mrow">
      <span className="meta"><span className="role">assistant</span>{metrics}</span>
      {streaming && <button className="Btn secondary sm" onClick={onCancel}><Icon name="square" size={12} /> Cancel</button>}
    </div>
  </div>
);

const KVvsPaged = () => (
  <ul>
    <li><b>KV cache</b> stores the key/value tensors for every prior token so the model never recomputes attention over the prompt — it trades GPU memory for decode speed.</li>
    <li><b>Fragmentation</b> is the catch: a contiguous per-sequence cache wastes VRAM as sequences grow and finish at different lengths.</li>
    <li><b>Paged attention</b> splits that cache into fixed-size blocks mapped through a lookup table, so memory is allocated on demand and shared across sequences — far less waste, higher batch concurrency.</li>
  </ul>
);

// shorter answer from the smaller/faster model
const KVvsPagedShort = () => (
  <ul>
    <li><b>KV cache</b> keeps past key/value tensors so tokens aren't recomputed.</li>
    <li>It wastes VRAM through fragmentation as sequences grow.</li>
    <li><b>Paged attention</b> stores it in fixed blocks via a lookup table — less waste, more concurrency.</li>
  </ul>
);

// ─── Error / stall / network message variants ────────────────────────
const ErrorMsg = ({ children, onRetry, role = "assistant" }) => (
  <div className="Msg assistant error">
    <div className="Bubble">{children}</div>
    <div className="mrow">
      <span className="meta"><span className="role">{role}</span><span>error</span></span>
      {onRetry && <button className="Btn secondary sm" onClick={onRetry}><Icon name="refresh" size={12} /> Retry</button>}
    </div>
  </div>
);

const StalledMsg = ({ onCancel, onRetry }) => (
  <div className="Msg assistant">
    <div className="Bubble">Paged attention builds on the KV cache concept by introducing a block-based memory management system that<span className="cursor" /></div>
    <div className="StallWarn" role="alert">
      <Icon name="alertTriangle" size={13} className="wi" />
      <span>Generation appears stalled — no tokens received for 30s.</span>
    </div>
    <div className="mrow">
      <span className="meta"><span className="role">assistant</span><span>TTFT 95ms</span><span>·</span><span>stalled</span></span>
      <span className="mrow" style={{ gap: 6 }}>
        <button className="Btn secondary sm" onClick={onCancel}><Icon name="square" size={12} /> Cancel</button>
        <button className="Btn secondary sm" onClick={onRetry}><Icon name="refresh" size={12} /> Retry</button>
      </span>
    </div>
  </div>
);

// ─── Comparison response pane (response only — input is shared) ───────
const ComparePane = ({ model, tag, side, onClose, variant }) => (
  <section className="PGChat compare-pane" aria-label={`Response from ${model}${side === "b" ? " (comparison)" : ""}`}>
    <div className="pc-head">
      <h3 className="mname" style={{ margin: 0, font: "var(--weight-semibold) 13px var(--font-mono)" }}>{model}</h3>
      <span className="mtag">{tag}</span>
      {side === "b" && <button className="DClose x" onClick={onClose} aria-label="Close comparison"><Icon name="x" size={15} /></button>}
    </div>
    <div className="PGScroll">
      <div className="PGMsgs">
        <UserMsg>Explain the difference between KV cache and paged attention in 3 bullet points.</UserMsg>
        {variant === "queued" ? (
          <div className="QueuedMsg" role="status">
            <div className="qm-spin"><span className="Spin" /></div>
            <div className="qm-body">
              <div className="qm-title">Queued — position <b>3</b></div>
              <div className="qm-sub">ETA ~8s · waiting for a free slot on gpu-1</div>
              <div className="qm-bar"><i style={{ width: "62%" }} /></div>
            </div>
            <button className="Btn secondary sm" onClick={() => {}}>Cancel</button>
          </div>
        ) : side === "b" ? (
          <AssistantMsg metrics={<><span>TTFT 35ms</span><span>·</span><span>85 tok/s</span><span>·</span><span>0.4s</span></>}>
            <KVvsPagedShort />
          </AssistantMsg>
        ) : (
          <AssistantMsg metrics={<><span>TTFT 120ms</span><span>·</span><span>42 tok/s</span><span>·</span><span>1.2s</span></>}>
            <KVvsPaged />
          </AssistantMsg>
        )}
      </div>
    </div>
  </section>
);

// ─── Input area ──────────────────────────────────────────────────────
const InputArea = ({ streaming, onStop, preserved, sharedHint }) => {
  const [sysOpen, setSysOpen] = useStatePG(false);
  const [draft, setDraft] = useStatePG(preserved || "");
  const send = () => {
    if (streaming || !draft.trim()) return;
    setDraft(""); // mock: clear the composer; real impl dispatches the turn
  };
  const onKeyDown = (e) => {
    if (e.key === "Enter" && !e.shiftKey) {
      e.preventDefault();        // don't insert a newline
      send();
    }
    // Shift+Enter falls through → textarea inserts a newline
  };
  return (
    <div className="PGInput">
      <div className="PGInputInner">
        <div className={`SysPrompt ${sysOpen ? "open" : ""}`}>
          <button className="sp-toggle" onClick={() => setSysOpen(o => !o)} aria-expanded={sysOpen}>
            <svg className="chev" viewBox="0 0 24 24"><polyline points="9 18 15 12 9 6" /></svg>
            System prompt (optional)
          </button>
          {sysOpen && (
            <div className="sp-body">
              <textarea placeholder="You are a helpful assistant specialized in systems programming…" defaultValue="" />
            </div>
          )}
        </div>
        <div className="Composer">
          <textarea
            placeholder="Type a message… (Enter to send, Shift+Enter for newline)"
            rows={1}
            value={draft}
            onChange={e => setDraft(e.target.value)}
            onKeyDown={onKeyDown}
          />
          {streaming
            ? <button className="Btn stop send" onClick={onStop}><Icon name="square" size={14} /> Stop</button>
            : <button className="Btn primary send" onClick={send}><Icon name="terminal" size={14} /> Send</button>}
        </div>
        {sharedHint && <div className="ComposerHint">{sharedHint}</div>}
      </div>
    </div>
  );
};

// ─── Chat pane (single model) ────────────────────────────────────────
const ChatPaneFull = ({ model, state, onCancel }) => (
  <section className="PGChat">
    <div className="pc-head">
      <span className="mname">{model}</span>
      <span className="mtag">· gpu-0 · slot-2</span>
    </div>
    <div className="PGScroll">
      {state === "empty" ? (
        <div className="PGEmpty">
          <div className="ic"><Icon name="terminal" size={24} /></div>
          <div className="t">Send a message to start testing</div>
          <div className="s">Enter to send · Shift+Enter for newline</div>
        </div>
      ) : (
        <div className="PGMsgs">
          <UserMsg>Explain the difference between KV cache and paged attention in 3 bullet points.</UserMsg>
          <AssistantMsg metrics={<><span>TTFT 120ms</span><span>·</span><span>42 tok/s</span><span>·</span><span>340 tokens</span><span>·</span><span>1.2s</span></>}>
            <KVvsPaged />
          </AssistantMsg>
          <AssistantMsg
            streaming
            onCancel={onCancel}
            metrics={<><span>TTFT 95ms</span><span>·</span><span>45 tok/s</span><span>·</span><span>streaming…</span></>}>
            <p>Paged attention builds on the KV cache concept by introducing a block-based memory management system that</p>
          </AssistantMsg>
        </div>
      )}
    </div>
    <InputArea streaming={state === "streaming"} onStop={onCancel} />
  </section>
);

// ─── Error chat pane ─────────────────────────────────────────────────
const ErrorChatPane = ({ model, variant }) => (
  <section className="PGChat">
    <div className="pc-head">
      <span className="mname">{model}</span>
      <span className="mtag">{variant === "unavailable" ? "· gpu-0 · failed" : "· gpu-0 · slot-2"}</span>
    </div>
    <div className="PGScroll">
      <div className="PGMsgs">
        <UserMsg>Explain the difference between KV cache and paged attention in 3 bullet points.</UserMsg>
        {variant === "unavailable" && (
          <ErrorMsg onRetry={() => {}}>
            Model unavailable: server <span style={{ fontFamily: "var(--font-mono)" }}>llama-70b-gpu0</span> is in <b>Failed</b> state. Start or restart it from the server detail page, then retry.
          </ErrorMsg>
        )}
        {variant === "stalled" && <StalledMsg onCancel={() => {}} onRetry={() => {}} />}
        {variant === "network" && (
          <ErrorMsg onRetry={() => {}}>
            Failed to send message — network connection lost. Your message is preserved in the composer below; retry when you’re back online.
          </ErrorMsg>
        )}
      </div>
    </div>
    <InputArea
      streaming={false}
      onStop={() => {}}
      preserved={variant === "network" ? "Explain the difference between KV cache and paged attention in 3 bullet points." : ""}
    />
  </section>
);

// ─── Comparison: split (≥1280) ───────────────────────────────────────
const CompareSplit = ({ rightVariant }) => (
  <div className="PGCompareArea">
    <div className="PGPanes">
      <ComparePane model="Llama 3.1 70B · Q4_K_M" tag="· gpu-0 · slot-2" side="a" />
      <ComparePane model="Llama 3.1 8B · Q8_0" tag="· gpu-1 · slot-0" side="b" variant={rightVariant} onClose={() => {}} />
    </div>
    <InputArea streaming={false} onStop={() => {}} sharedHint="Sends to both models" />
  </div>
);

// ─── Comparison: tabbed (≤1279) ──────────────────────────────────────
const CompareTabs = ({ rightVariant }) => {
  const [active, setActive] = useStatePG("a");
  return (
    <div className="PGCompareArea">
      <div className="PGTabs" role="tablist">
        <button role="tab" aria-selected={active === "a"} className={`pgtab ${active === "a" ? "on" : ""}`} onClick={() => setActive("a")}>Llama 3.1 70B</button>
        <button role="tab" aria-selected={active === "b"} className={`pgtab ${active === "b" ? "on" : ""}`} onClick={() => setActive("b")}>Llama 3.1 8B</button>
      </div>
      {active === "a"
        ? <ComparePane model="Llama 3.1 70B · Q4_K_M" tag="· gpu-0 · slot-2" side="a" />
        : <ComparePane model="Llama 3.1 8B · Q8_0" tag="· gpu-1 · slot-0" side="b" variant={rightVariant} onClose={() => {}} />}
      <InputArea streaming={false} onStop={() => {}} sharedHint="Sends to both models" />
    </div>
  );
};

// ─── Page ────────────────────────────────────────────────────────────
const PlaygroundPage = ({ state = "empty", model = PG_MODELS[0], mode }) => {
  if (mode === "compare-split" || mode === "compare-tabs") {
    return (
      <div className="PG compare">
        <ParamSidebar compare onCompare={() => {}} />
        {mode === "compare-split"
          ? <CompareSplit rightVariant={state === "queued" ? "queued" : null} />
          : <CompareTabs rightVariant={state === "queued" ? "queued" : null} />}
      </div>
    );
  }
  if (mode === "error") {
    return (
      <div className="PG">
        <ParamSidebar onCompare={() => {}} />
        <ErrorChatPane model={model} variant={state} />
      </div>
    );
  }
  return (
    <div className="PG">
      <ParamSidebar onCompare={() => {}} />
      <ChatPaneFull model={model} state={state} onCancel={() => {}} />
    </div>
  );
};

Object.assign(window, { PlaygroundPage });

})();
