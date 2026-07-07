import React, { useCallback, useEffect, useMemo, useRef, useState } from "react";
import { createRoot } from "react-dom/client";
import { invoke } from "@tauri-apps/api/core";
import "./styles.css";

/* ─── Types ────────────────────────────────────────────────────────── */
type ProjectEntry = {
  name: string;
  root: string;
  config: string;
  backend: string;
};

type ServiceEntry = {
  name: string;
  image: string;
  backend: string;
  ports: string[];
  urls: string[];
  endpoints: EndpointEntry[];
};

type EndpointEntry = {
  host_port: number;
  target_port: number;
  protocol: string;
  label: string;
  url: string | null;
  browser_safe: boolean;
};

type RuntimeOverview = {
  containers: string;
  images: string;
  volumes: string;
  networks: string;
  vm_health: string;
};

type CommandResult = {
  exit_code: number;
  stdout: string;
  stderr: string;
  timed_out: boolean;
};

type SystemStats = {
  cpu_usage: number;
  cpu_cores: number[];
  used_memory: number;
  total_memory: number;
  cpu_count: number;
  unavailable?: boolean;
};

type Tab = "Services" | "Containers" | "Images" | "Volumes" | "Networks" | "VM" | "Logs" | "Settings";
type ThemePreference = "system" | "light" | "dark";
type ServiceColumnKey = "service" | "image" | "ports" | "open" | "controls" | "logs";
type AppSettings = {
  theme: ThemePreference;
  autoRefreshSeconds: number;
  statsRefreshSeconds: number;
  showSystemMetrics: boolean;
  compactRows: boolean;
  openLogsOnFailure: boolean;
  openLogsForCommandOutput: boolean;
};

/* ─── Constants ────────────────────────────────────────────────────── */
const tabs: Tab[] = ["Services", "Containers", "Images", "Volumes", "Networks", "VM", "Logs", "Settings"];
const appSettingsStorageKey = "stackdeck.settings.v2";
const serviceColumnStorageKey = "stackdeck.serviceColumns";
const defaultAppSettings: AppSettings = {
  theme: "system",
  autoRefreshSeconds: 0,
  statsRefreshSeconds: 2,
  showSystemMetrics: true,
  compactRows: false,
  openLogsOnFailure: false,
  openLogsForCommandOutput: false,
};
const serviceColumns: { key: ServiceColumnKey; label: string }[] = [
  { key: "service", label: "Service" },
  { key: "image", label: "Image" },
  { key: "ports", label: "Ports" },
  { key: "open", label: "Open" },
  { key: "controls", label: "Controls" },
  { key: "logs", label: "Logs" },
];
const serviceColumnDefaults: Record<ServiceColumnKey, number> = {
  service: 164,
  image: 240,
  ports: 148,
  open: 140,
  controls: 218,
  logs: 88,
};
const serviceColumnMinimums: Record<ServiceColumnKey, number> = {
  service: 130,
  image: 150,
  ports: 120,
  open: 110,
  controls: 180,
  logs: 76,
};
const emptyOverview: RuntimeOverview = {
  containers: "",
  images: "",
  volumes: "",
  networks: "",
  vm_health: "",
};

/* ─── SVG Icons ────────────────────────────────────────────────────── */
/* ─── Brand Glyph ─────────────────────────────────────────────────── */
function BrandGlyph() {
  return (
    <svg
      viewBox="0 0 36 36"
      fill="none"
      xmlns="http://www.w3.org/2000/svg"
      style={{ width: 36, height: 36, display: "block" }}
      aria-hidden="true"
    >
      <defs>
        {/* Primary emerald gradient */}
        <linearGradient id="sg-primary" x1="4" y1="5" x2="32" y2="31" gradientUnits="userSpaceOnUse">
          <stop stopColor="#6ee7b7" />
          <stop offset="1" stopColor="#059669" />
        </linearGradient>
        {/* Mid-layer gradient */}
        <linearGradient id="sg-mid" x1="4" y1="14" x2="32" y2="31" gradientUnits="userSpaceOnUse">
          <stop stopColor="#34d399" stopOpacity="0.72" />
          <stop offset="1" stopColor="#047857" stopOpacity="0.72" />
        </linearGradient>
        {/* Bottom-layer gradient */}
        <linearGradient id="sg-low" x1="4" y1="23" x2="32" y2="31" gradientUnits="userSpaceOnUse">
          <stop stopColor="#34d399" stopOpacity="0.38" />
          <stop offset="1" stopColor="#047857" stopOpacity="0.38" />
        </linearGradient>
        {/* Glow filter */}
        <filter id="sg-glow" x="-30%" y="-30%" width="160%" height="160%">
          <feGaussianBlur stdDeviation="1.5" result="blur" />
          <feMerge>
            <feMergeNode in="blur" />
            <feMergeNode in="SourceGraphic" />
          </feMerge>
        </filter>
      </defs>

      {/* ── Bottom slab — offset right, dimmest ── */}
      <rect
        x="9" y="23.5" width="22" height="7" rx="3"
        fill="url(#sg-low)"
      />

      {/* ── Middle slab — slight offset right ── */}
      <rect
        x="6" y="14.5" width="24" height="7" rx="3"
        fill="url(#sg-mid)"
      />

      {/* ── Top slab — front, full brightness ── */}
      <rect
        x="4" y="5.5" width="24" height="7" rx="3"
        fill="url(#sg-primary)"
        filter="url(#sg-glow)"
      />

      {/* Inner shine on top slab */}
      <rect
        x="5" y="6.2" width="16" height="2.2" rx="1.1"
        fill="white" fillOpacity="0.28"
      />

      {/* Tiny right-edge accent dot on top slab */}
      <circle cx="25" cy="9" r="1.5" fill="white" fillOpacity="0.35" />
    </svg>
  );
}

function IconLayers({ size = 16 }: { size?: number }) {
  return (
    <svg width={size} height={size} viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="2" strokeLinecap="round" strokeLinejoin="round">
      <polygon points="12 2 2 7 12 12 22 7 12 2"/>
      <polyline points="2 17 12 22 22 17"/>
      <polyline points="2 12 12 17 22 12"/>
    </svg>
  );
}

function IconBox({ size = 16 }: { size?: number }) {
  return (
    <svg width={size} height={size} viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="2" strokeLinecap="round" strokeLinejoin="round">
      <path d="M21 16V8a2 2 0 0 0-1-1.73l-7-4a2 2 0 0 0-2 0l-7 4A2 2 0 0 0 3 8v8a2 2 0 0 0 1 1.73l7 4a2 2 0 0 0 2 0l7-4A2 2 0 0 0 21 16z"/>
    </svg>
  );
}

function IconImage({ size = 16 }: { size?: number }) {
  return (
    <svg width={size} height={size} viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="2" strokeLinecap="round" strokeLinejoin="round">
      <rect x="3" y="3" width="18" height="18" rx="2" ry="2"/>
      <circle cx="8.5" cy="8.5" r="1.5"/>
      <polyline points="21 15 16 10 5 21"/>
    </svg>
  );
}

function IconHardDrive({ size = 16 }: { size?: number }) {
  return (
    <svg width={size} height={size} viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="2" strokeLinecap="round" strokeLinejoin="round">
      <line x1="22" y1="12" x2="2" y2="12"/>
      <path d="M5.45 5.11L2 12v6a2 2 0 0 0 2 2h16a2 2 0 0 0 2-2v-6l-3.45-6.89A2 2 0 0 0 16.76 4H7.24a2 2 0 0 0-1.79 1.11z"/>
      <line x1="6" y1="16" x2="6.01" y2="16"/>
      <line x1="10" y1="16" x2="10.01" y2="16"/>
    </svg>
  );
}

function IconShare({ size = 16 }: { size?: number }) {
  return (
    <svg width={size} height={size} viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="2" strokeLinecap="round" strokeLinejoin="round">
      <circle cx="18" cy="5" r="3"/>
      <circle cx="6" cy="12" r="3"/>
      <circle cx="18" cy="19" r="3"/>
      <line x1="8.59" y1="13.51" x2="15.42" y2="17.49"/>
      <line x1="15.41" y1="6.51" x2="8.59" y2="10.49"/>
    </svg>
  );
}

function IconCpu({ size = 16 }: { size?: number }) {
  return (
    <svg width={size} height={size} viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="2" strokeLinecap="round" strokeLinejoin="round">
      <rect x="4" y="4" width="16" height="16" rx="2" ry="2"/>
      <rect x="9" y="9" width="6" height="6"/>
      <line x1="9" y1="1" x2="9" y2="4"/>
      <line x1="15" y1="1" x2="15" y2="4"/>
      <line x1="9" y1="20" x2="9" y2="23"/>
      <line x1="15" y1="20" x2="15" y2="23"/>
      <line x1="20" y1="9" x2="23" y2="9"/>
      <line x1="20" y1="14" x2="23" y2="14"/>
      <line x1="1" y1="9" x2="4" y2="9"/>
      <line x1="1" y1="14" x2="4" y2="14"/>
    </svg>
  );
}

function IconTerminal({ size = 16 }: { size?: number }) {
  return (
    <svg width={size} height={size} viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="2" strokeLinecap="round" strokeLinejoin="round">
      <polyline points="4 17 10 11 4 5"/>
      <line x1="12" y1="19" x2="20" y2="19"/>
    </svg>
  );
}

function IconRefresh({ size = 16 }: { size?: number }) {
  return (
    <svg width={size} height={size} viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="2" strokeLinecap="round" strokeLinejoin="round">
      <polyline points="23 4 23 10 17 10"/>
      <polyline points="1 20 1 14 7 14"/>
      <path d="M3.51 9a9 9 0 0 1 14.85-3.36L23 10M1 14l4.64 4.36A9 9 0 0 0 20.49 15"/>
    </svg>
  );
}

function IconPlay({ size = 13 }: { size?: number }) {
  return (
    <svg width={size} height={size} viewBox="0 0 24 24" fill="currentColor" stroke="none">
      <polygon points="5 3 19 12 5 21 5 3"/>
    </svg>
  );
}

function IconStop({ size = 12 }: { size?: number }) {
  return (
    <svg width={size} height={size} viewBox="0 0 24 24" fill="currentColor" stroke="none">
      <rect x="3" y="3" width="18" height="18" rx="2" ry="2"/>
    </svg>
  );
}

function IconTrash({ size = 12 }: { size?: number }) {
  return (
    <svg width={size} height={size} viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="2" strokeLinecap="round" strokeLinejoin="round">
      <polyline points="3 6 5 6 21 6"></polyline>
      <path d="M19 6v14a2 2 0 0 1-2 2H7a2 2 0 0 1-2-2V6m3 0V4a2 2 0 0 1 2-2h4a2 2 0 0 1 2 2v2"></path>
    </svg>
  );
}

function IconRotateCw({ size = 13 }: { size?: number }) {
  return (
    <svg width={size} height={size} viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="2.5" strokeLinecap="round" strokeLinejoin="round">
      <polyline points="23 4 23 10 17 10"/>
      <path d="M20.49 15a9 9 0 1 1-2.12-9.36L23 10"/>
    </svg>
  );
}

function IconFileText({ size = 13 }: { size?: number }) {
  return (
    <svg width={size} height={size} viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="2" strokeLinecap="round" strokeLinejoin="round">
      <path d="M14 2H6a2 2 0 0 0-2 2v16a2 2 0 0 0 2 2h12a2 2 0 0 0 2-2V8z"/>
      <polyline points="14 2 14 8 20 8"/>
      <line x1="16" y1="13" x2="8" y2="13"/>
      <line x1="16" y1="17" x2="8" y2="17"/>
      <polyline points="10 9 9 9 8 9"/>
    </svg>
  );
}

function IconExternalLink({ size = 11 }: { size?: number }) {
  return (
    <svg width={size} height={size} viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="2.5" strokeLinecap="round" strokeLinejoin="round">
      <path d="M18 13v6a2 2 0 0 1-2 2H5a2 2 0 0 1-2-2V8a2 2 0 0 1 2-2h6"/>
      <polyline points="15 3 21 3 21 9"/>
      <line x1="10" y1="14" x2="21" y2="3"/>
    </svg>
  );
}

function IconSun({ size = 14 }: { size?: number }) {
  return (
    <svg width={size} height={size} viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="2" strokeLinecap="round" strokeLinejoin="round">
      <circle cx="12" cy="12" r="5"/>
      <line x1="12" y1="1" x2="12" y2="3"/>
      <line x1="12" y1="21" x2="12" y2="23"/>
      <line x1="4.22" y1="4.22" x2="5.64" y2="5.64"/>
      <line x1="18.36" y1="18.36" x2="19.78" y2="19.78"/>
      <line x1="1" y1="12" x2="3" y2="12"/>
      <line x1="21" y1="12" x2="23" y2="12"/>
      <line x1="4.22" y1="19.78" x2="5.64" y2="18.36"/>
      <line x1="18.36" y1="5.64" x2="19.78" y2="4.22"/>
    </svg>
  );
}

function IconSettings({ size = 16 }: { size?: number }) {
  return (
    <svg width={size} height={size} viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="2" strokeLinecap="round" strokeLinejoin="round">
      <path d="M12.22 2h-.44a2 2 0 0 0-2 2v.18a2 2 0 0 1-1 1.73l-.43.25a2 2 0 0 1-2 0l-.15-.08a2 2 0 0 0-2.73.73l-.22.38a2 2 0 0 0 .73 2.73l.15.1a2 2 0 0 1 1 1.72v.51a2 2 0 0 1-1 1.73l-.15.09a2 2 0 0 0-.73 2.73l.22.38a2 2 0 0 0 2.73.73l.15-.08a2 2 0 0 1 2 0l.43.25a2 2 0 0 1 1 1.73V20a2 2 0 0 0 2 2h.44a2 2 0 0 0 2-2v-.18a2 2 0 0 1 1-1.73l.43-.25a2 2 0 0 1 2 0l.15.08a2 2 0 0 0 2.73-.73l.22-.38a2 2 0 0 0-.73-2.73l-.15-.09a2 2 0 0 1-1-1.73v-.51a2 2 0 0 1 1-1.72l.15-.1a2 2 0 0 0 .73-2.73l-.22-.38a2 2 0 0 0-2.73-.73l-.15.08a2 2 0 0 1-2 0l-.43-.25a2 2 0 0 1-1-1.73V4a2 2 0 0 0-2-2z"/>
      <circle cx="12" cy="12" r="3"/>
    </svg>
  );
}

function IconInbox({ size = 28 }: { size?: number }) {
  return (
    <svg width={size} height={size} viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="1.5" strokeLinecap="round" strokeLinejoin="round">
      <polyline points="22 12 16 12 14 15 10 15 8 12 2 12"/>
      <path d="M5.45 5.11L2 12v6a2 2 0 0 0 2 2h16a2 2 0 0 0 2-2v-6l-3.45-6.89A2 2 0 0 0 16.76 4H7.24a2 2 0 0 0-1.79 1.11z"/>
    </svg>
  );
}

function IconArrowLeft({ size = 14 }: { size?: number }) {
  return (
    <svg width={size} height={size} viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="2.5" strokeLinecap="round" strokeLinejoin="round">
      <line x1="19" y1="12" x2="5" y2="12" />
      <polyline points="12 19 5 12 12 5" />
    </svg>
  );
}

const tabIcons: Record<Tab, React.ReactNode> = {
  Services:   <IconLayers size={15} />,
  Containers: <IconBox size={15} />,
  Images:     <IconImage size={15} />,
  Volumes:    <IconHardDrive size={15} />,
  Networks:   <IconShare size={15} />,
  VM:         <IconCpu size={15} />,
  Logs:       <IconTerminal size={15} />,
  Settings:   <IconSettings size={15} />,
};

/* ─── Helpers ──────────────────────────────────────────────────────── */
function formatBytes(bytes: number): string {
  if (bytes === 0) return "0 B";
  const gb = bytes / (1024 ** 3);
  if (gb >= 1) return `${gb.toFixed(1)} GB`;
  const mb = bytes / (1024 ** 2);
  return `${mb.toFixed(0)} MB`;
}

function clamp(v: number, lo: number, hi: number) {
  return Math.max(lo, Math.min(hi, v));
}

/* ─── useSystemStats hook ───────────────────────────────────────────── */
const EMPTY_STATS: SystemStats = {
  cpu_usage: 0, cpu_cores: [], used_memory: 0, total_memory: 0, cpu_count: 0,
};

function useSystemStats(intervalMs = 2000) {
  const [stats, setStats] = useState<SystemStats>(EMPTY_STATS);
  const [history, setHistory] = useState<{ cpu: number[]; ram: number[] }>(
    { cpu: Array(20).fill(0), ram: Array(20).fill(0) },
  );

  useEffect(() => {
    let cancelled = false;

    async function fetchStats() {
      try {
        const s = await invoke<SystemStats>("system_stats");
        if (!cancelled) {
          setStats(s);
          setHistory((h) => ({
            cpu: [...h.cpu.slice(1), clamp(s.cpu_usage, 0, 100)],
            ram: [...h.ram.slice(1), s.total_memory > 0
              ? clamp((s.used_memory / s.total_memory) * 100, 0, 100)
              : 0],
          }));
        }
      } catch {
        if (!cancelled) {
          setStats({ ...EMPTY_STATS, unavailable: true });
        }
      }
    }

    fetchStats();
    const id = window.setInterval(fetchStats, intervalMs);
    return () => { cancelled = true; clearInterval(id); };
  }, [intervalMs]); // eslint-disable-line react-hooks/exhaustive-deps

  return { stats, history };
}

/* ─── Sparkline SVG ─────────────────────────────────────────────────── */
function Sparkline({ values, color }: { values: number[]; color: string }) {
  const W = 80, H = 28;
  const max = Math.max(...values, 1);
  const pts = values.map((v, i) => {
    const x = (i / (values.length - 1)) * W;
    const y = H - (v / max) * (H - 2) - 1;
    return `${x},${y}`;
  }).join(" ");
  const fill = `${pts} ${W},${H} 0,${H}`;
  return (
    <svg viewBox={`0 0 ${W} ${H}`} style={{ width: "100%", height: H, display: "block", overflow: "visible" }}>
      <defs>
        <linearGradient id={`spark-${color.replace("#","")}`} x1="0" y1="0" x2="0" y2="1">
          <stop offset="0%" stopColor={color} stopOpacity="0.35" />
          <stop offset="100%" stopColor={color} stopOpacity="0.02" />
        </linearGradient>
      </defs>
      <polygon points={fill} fill={`url(#spark-${color.replace("#","")})`} />
      <polyline points={pts} fill="none" stroke={color} strokeWidth="1.5" strokeLinejoin="round" strokeLinecap="round" />
    </svg>
  );
}

/* ─── Arc Gauge ─────────────────────────────────────────────────────── */
function ArcGauge({ pct, color, size = 52 }: { pct: number; color: string; size?: number }) {
  const r = (size / 2) - 5;
  const circ = Math.PI * r; // half-circle arc
  const dash = clamp(pct / 100, 0, 1) * circ;
  const cx = size / 2, cy = size / 2;
  return (
    <svg width={size} height={size / 2 + 6} viewBox={`0 0 ${size} ${size / 2 + 6}`} style={{ overflow: "visible" }}>
      {/* Track */}
      <path
        d={`M ${5} ${cy} A ${r} ${r} 0 0 1 ${size - 5} ${cy}`}
        fill="none" stroke="currentColor" strokeOpacity="0.1" strokeWidth="4" strokeLinecap="round"
      />
      {/* Fill */}
      <path
        d={`M ${5} ${cy} A ${r} ${r} 0 0 1 ${size - 5} ${cy}`}
        fill="none" stroke={color} strokeWidth="4" strokeLinecap="round"
        strokeDasharray={`${dash} ${circ}`}
        style={{ transition: "stroke-dasharray 600ms cubic-bezier(0.22,1,0.36,1)" }}
      />
    </svg>
  );
}

/* ─── CPU Card ──────────────────────────────────────────────────────── */
function CpuCard({ stats, history }: { stats: SystemStats; history: number[] }) {
  if (stats.unavailable) {
    return (
      <article className="metric-card sys-card" aria-label="CPU usage">
        <div className="sys-card-header">
          <span className="metric-label">CPU</span>
          <span className="sys-card-badge" style={{ color: "#9ca3af" }}>Unavailable</span>
        </div>
        <div className="sys-card-gauge-row">
          <div className="sys-card-gauge">
            <ArcGauge pct={0} color="#4b5563" size={60} />
            <span className="sys-card-gauge-label" style={{ color: "#9ca3af" }}>—</span>
          </div>
          <div className="sys-card-spark">
            <Sparkline values={history} color="#4b5563" />
            <span className="sys-card-sub">—</span>
          </div>
        </div>
      </article>
    );
  }

  const pct = clamp(stats.cpu_usage, 0, 100);
  const color = pct > 80 ? "var(--danger)" : pct > 60 ? "var(--warn)" : "var(--accent-glow)";
  const sparkColor = pct > 80 ? "#f87171" : pct > 60 ? "#fbbf24" : "#34d399";
  return (
    <article className="metric-card sys-card" aria-label="CPU usage">
      <div className="sys-card-header">
        <span className="metric-label">CPU</span>
        <span className="sys-card-badge" style={{ color }}>{pct.toFixed(1)}%</span>
      </div>
      <div className="sys-card-gauge-row">
        <div className="sys-card-gauge">
          <ArcGauge pct={pct} color={sparkColor} size={60} />
          <span className="sys-card-gauge-label" style={{ color }}>{Math.round(pct)}%</span>
        </div>
        <div className="sys-card-spark">
          <Sparkline values={history} color={sparkColor} />
          {stats.cpu_count > 0 && (
            <span className="sys-card-sub">{stats.cpu_count} cores</span>
          )}
        </div>
      </div>
    </article>
  );
}

/* ─── RAM Card ──────────────────────────────────────────────────────── */
function RamCard({ stats, history }: { stats: SystemStats; history: number[] }) {
  if (stats.unavailable) {
    return (
      <article className="metric-card sys-card" aria-label="Memory usage">
        <div className="sys-card-header">
          <span className="metric-label">RAM</span>
          <span className="sys-card-badge" style={{ color: "#9ca3af" }}>Unavailable</span>
        </div>
        <div className="sys-card-gauge-row">
          <div className="sys-card-gauge">
            <ArcGauge pct={0} color="#4b5563" size={60} />
            <span className="sys-card-gauge-label" style={{ color: "#9ca3af" }}>—</span>
          </div>
          <div className="sys-card-spark">
            <Sparkline values={history} color="#4b5563" />
            <span className="sys-card-sub">—</span>
          </div>
        </div>
      </article>
    );
  }

  const pct = stats.total_memory > 0
    ? clamp((stats.used_memory / stats.total_memory) * 100, 0, 100)
    : 0;
  const color = pct > 85 ? "var(--danger)" : pct > 65 ? "var(--warn)" : "hsl(217 91% 65%)";
  const sparkColor = pct > 85 ? "#f87171" : pct > 65 ? "#fbbf24" : "#60a5fa";
  return (
    <article className="metric-card sys-card" aria-label="Memory usage">
      <div className="sys-card-header">
        <span className="metric-label">RAM</span>
        <span className="sys-card-badge" style={{ color }}>{pct.toFixed(1)}%</span>
      </div>
      <div className="sys-card-gauge-row">
        <div className="sys-card-gauge">
          <ArcGauge pct={pct} color={sparkColor} size={60} />
          <span className="sys-card-gauge-label" style={{ color }}>
            {formatBytes(stats.used_memory)}
          </span>
        </div>
        <div className="sys-card-spark">
          <Sparkline values={history} color={sparkColor} />
          {stats.total_memory > 0 && (
            <span className="sys-card-sub">of {formatBytes(stats.total_memory)}</span>
          )}
        </div>
      </div>
    </article>
  );
}

function labelAction(action: string) {
  switch (action) {
    case "up":   return "Start";
    case "down": return "Stop";
    default:     return action.charAt(0).toUpperCase() + action.slice(1);
  }
}

function formatError(error: unknown) {
  return error instanceof Error ? error.message : String(error);
}

function formatCommandStart(
  action: string,
  project: string,
  target: string,
  config: string,
  backend: string,
) {
  return [
    `command=${action}`,
    `project=${project}`,
    `target=${target}`,
    `backend=${backend}`,
    `config=${config}`,
    `started=${new Date().toLocaleString()}`,
  ].join("\n");
}

function readAppSettings(): AppSettings {
  const stored = localStorage.getItem(appSettingsStorageKey);
  if (!stored) {
    const oldStored = localStorage.getItem("stackdeck.settings");
    if (oldStored) {
      try {
        const oldParsed = JSON.parse(oldStored) as Partial<AppSettings>;
        return {
          theme:
            oldParsed.theme === "light" || oldParsed.theme === "dark" || oldParsed.theme === "system"
              ? oldParsed.theme
              : "system",
          autoRefreshSeconds: [0, 5, 10, 30, 60].includes(Number(oldParsed.autoRefreshSeconds))
            ? Number(oldParsed.autoRefreshSeconds)
            : defaultAppSettings.autoRefreshSeconds,
          statsRefreshSeconds: [1, 2, 5, 10].includes(Number(oldParsed.statsRefreshSeconds))
            ? Number(oldParsed.statsRefreshSeconds)
            : defaultAppSettings.statsRefreshSeconds,
          showSystemMetrics: oldParsed.showSystemMetrics ?? defaultAppSettings.showSystemMetrics,
          compactRows: oldParsed.compactRows ?? defaultAppSettings.compactRows,
          openLogsOnFailure: defaultAppSettings.openLogsOnFailure,
          openLogsForCommandOutput: defaultAppSettings.openLogsForCommandOutput,
        };
      } catch {
        // Fall back to default
      }
    }
    return { ...defaultAppSettings, theme: "system" };
  }
  try {
    const parsed = JSON.parse(stored) as Partial<AppSettings>;
    return {
      theme:
        parsed.theme === "light" || parsed.theme === "dark" || parsed.theme === "system"
          ? parsed.theme
          : "system",
      autoRefreshSeconds: [0, 5, 10, 30, 60].includes(Number(parsed.autoRefreshSeconds))
        ? Number(parsed.autoRefreshSeconds)
        : defaultAppSettings.autoRefreshSeconds,
      statsRefreshSeconds: [1, 2, 5, 10].includes(Number(parsed.statsRefreshSeconds))
        ? Number(parsed.statsRefreshSeconds)
        : defaultAppSettings.statsRefreshSeconds,
      showSystemMetrics: parsed.showSystemMetrics ?? defaultAppSettings.showSystemMetrics,
      compactRows: parsed.compactRows ?? defaultAppSettings.compactRows,
      openLogsOnFailure: parsed.openLogsOnFailure ?? defaultAppSettings.openLogsOnFailure,
      openLogsForCommandOutput:
        parsed.openLogsForCommandOutput ?? defaultAppSettings.openLogsForCommandOutput,
    };
  } catch {
    return { ...defaultAppSettings, theme: "system" };
  }
}

function readServiceColumnWidths(): Record<ServiceColumnKey, number> {
  const stored = localStorage.getItem(serviceColumnStorageKey);
  if (!stored) return serviceColumnDefaults;
  try {
    const parsed = JSON.parse(stored) as Partial<Record<ServiceColumnKey, number>>;
    return serviceColumns.reduce(
      (widths, column) => ({
        ...widths,
        [column.key]: Math.max(
          serviceColumnMinimums[column.key],
          Number(parsed[column.key]) || serviceColumnDefaults[column.key],
        ),
      }),
      {} as Record<ServiceColumnKey, number>,
    );
  } catch {
    return serviceColumnDefaults;
  }
}

function resetServiceColumnWidths() {
  localStorage.removeItem(serviceColumnStorageKey);
  window.dispatchEvent(new CustomEvent("stackdeck:reset-service-columns"));
}

/* ─── Sub-Components ───────────────────────────────────────────────── */

function MetricCard({ label, value, text = false }: { label: string; value: number | string; text?: boolean }) {
  return (
    <article className="metric-card">
      <span className="metric-label">{label}</span>
      <strong className={`metric-value${text ? " metric-text" : ""}`}>{value}</strong>
    </article>
  );
}

function MetaPill({ label, value }: { label: string; value: string }) {
  return (
    <div className="meta-pill" title={value}>
      <span className="meta-label">{label}</span>
      <span className="meta-value truncate">{value || "—"}</span>
    </div>
  );
}

function TextPanel({ title, text, onBack }: { title: string; text: string; onBack?: () => void }) {
  const isLoaded = Boolean(text);
  return (
    <section className="text-panel">
      <div className="panel-header">
        <div style={{ display: "flex", alignItems: "center", gap: "10px" }}>
          {onBack && (
            <button className="btn-back" onClick={onBack} title="Back to Services" type="button">
              <IconArrowLeft size={12} />
              Back
            </button>
          )}
          <h2>{title}</h2>
        </div>
        <span className={`panel-status-badge${isLoaded ? " loaded" : ""}`}>
          {isLoaded ? "Loaded" : "Empty"}
        </span>
      </div>
      <pre>{text || "No data loaded."}</pre>
    </section>
  );
}

/* ─── Services View ────────────────────────────────────────────────── */
function ServicesView({
  services,
  pendingAction,
  onAction,
  onOpen,
}: {
  services: ServiceEntry[];
  pendingAction: string;
  onAction: (action: string, serviceName?: string) => void;
  onOpen: (serviceName: string, hostPort: number) => void;
}) {
  const [columnWidths, setColumnWidths] = useState<Record<ServiceColumnKey, number>>(() =>
    readServiceColumnWidths(),
  );
  const resizeState = useRef<{
    key: ServiceColumnKey;
    startX: number;
    startWidth: number;
    onPointerMove: (e: PointerEvent) => void;
    onPointerUp: () => void;
  } | null>(null);

  useEffect(() => {
    return () => {
      const cur = resizeState.current;
      if (cur) {
        window.removeEventListener("pointermove", cur.onPointerMove);
        window.removeEventListener("pointerup", cur.onPointerUp);
      }
    };
  }, []);

  useEffect(() => {
    const onReset = () => setColumnWidths(serviceColumnDefaults);
    window.addEventListener("stackdeck:reset-service-columns", onReset);
    return () => window.removeEventListener("stackdeck:reset-service-columns", onReset);
  }, []);

  const gridTemplateColumns = serviceColumns.map((col) => `${columnWidths[col.key]}px`).join(" ");
  const gridStyle = { gridTemplateColumns } as React.CSSProperties;

  function startColumnResize(key: ServiceColumnKey, event: React.PointerEvent<HTMLButtonElement>) {
    event.preventDefault();
    event.currentTarget.setPointerCapture(event.pointerId);

    const onPointerMove = (e: PointerEvent) => {
      const cur = resizeState.current;
      if (!cur) return;
      const nextWidth = Math.max(serviceColumnMinimums[cur.key], cur.startWidth + e.clientX - cur.startX);
      setColumnWidths((w) => ({ ...w, [cur.key]: nextWidth }));
    };
    const onPointerUp = () => {
      setColumnWidths((w) => {
        localStorage.setItem(serviceColumnStorageKey, JSON.stringify(w));
        return w;
      });
      resizeState.current = null;
      window.removeEventListener("pointermove", onPointerMove);
      window.removeEventListener("pointerup", onPointerUp);
    };

    resizeState.current = { key, startX: event.clientX, startWidth: columnWidths[key], onPointerMove, onPointerUp };
    window.addEventListener("pointermove", onPointerMove);
    window.addEventListener("pointerup", onPointerUp);
  }

  if (services.length === 0) {
    return (
      <section className="empty-state">
        <div className="empty-state-icon">
          <IconInbox size={26} />
        </div>
        <h2>No services found</h2>
        <p>Register a project with a stack.json or Compose YAML config to populate this dashboard.</p>
      </section>
    );
  }

  return (
    <section className="service-table">
      <div className="table-head" style={gridStyle}>
        {serviceColumns.map((col, index) => (
          <span className="column-heading" key={col.key}>
            {col.label}
            {index < serviceColumns.length - 1 && (
              <button
                aria-label={`Resize ${col.label} column`}
                className="column-resizer"
                onPointerDown={(e) => startColumnResize(col.key, e)}
                type="button"
              />
            )}
          </span>
        ))}
      </div>

      {services.map((svc) => (
        <article className="service-row" key={svc.name} style={gridStyle}>
          {/* Service Name */}
          <div className="service-name-cell">
            <strong>{svc.name}</strong>
            <div className="svc-endpoints">
              <span className={`svc-dot${svc.endpoints.length > 0 ? " active" : ""}`} />
              {svc.endpoints.length} endpoint{svc.endpoints.length !== 1 ? "s" : ""}
            </div>
          </div>

          {/* Image */}
          <span className="image-cell truncate" title={svc.image || "—"}>
            {svc.image || "—"}
          </span>

          {/* Ports */}
          <span className="ports-cell truncate" title={svc.ports.join(", ") || "—"}>
            {svc.ports.length ? svc.ports.join(", ") : "—"}
          </span>

          {/* Endpoints / Open */}
          <div className="endpoints-cell">
            {svc.endpoints.length === 0 && <span style={{ color: "var(--text-tertiary)", fontSize: 12 }}>—</span>}
            {svc.endpoints.map((ep) => {
              const key = `${ep.host_port}-${ep.target_port}-${ep.label}`;
              if (ep.url) {
                const url = ep.url;
                return (
                  <button
                    key={key}
                    className="endpoint-link"
                    onClick={() => onOpen(svc.name, ep.host_port)}
                    title={`${ep.label} — ${url}`}
                  >
                    :{ep.host_port}
                    <IconExternalLink size={10} />
                  </button>
                );
              }
              return (
                <span className="endpoint-chip" key={key} title={`${ep.label} on ${ep.host_port}:${ep.target_port}`}>
                  {ep.label}&nbsp;<span style={{ opacity: 0.6 }}>:{ep.host_port}</span>
                </span>
              );
            })}
          </div>

          {/* Controls */}
          <div className="controls-cell">
            <button
              className="btn-svc-start"
              disabled={pendingAction.includes(":" + svc.name) || pendingAction.includes(":all services")}
              onClick={() => onAction("up", svc.name)}
              title="Start service"
            >
              <IconPlay size={11} />
              Start
            </button>
            <button
              className="btn-svc-restart"
              disabled={pendingAction.includes(":" + svc.name) || pendingAction.includes(":all services")}
              onClick={() => onAction("restart", svc.name)}
              title="Restart service"
            >
              <IconRotateCw size={11} />
              Restart
            </button>
            <button
              className="btn-svc-stop"
              disabled={pendingAction.includes(":" + svc.name) || pendingAction.includes(":all services")}
              onClick={() => onAction("down", svc.name)}
              title="Stop service"
            >
              <IconStop size={10} />
              Stop
            </button>
          </div>

          {/* Logs */}
          <div className="logs-cell">
            <button
              className="btn-logs"
              disabled={pendingAction.includes(":" + svc.name) || pendingAction.includes(":all services")}
              onClick={() => onAction("logs", svc.name)}
              title="View logs"
            >
              <IconFileText size={12} />
              Logs
            </button>
          </div>
        </article>
      ))}
    </section>
  );
}

function SettingsView({
  settings,
  onSettingsChange,
  onResetColumns,
  onRefresh,
  refreshing,
}: {
  settings: AppSettings;
  onSettingsChange: (next: AppSettings) => void;
  onResetColumns: () => void;
  onRefresh: () => void;
  refreshing: boolean;
}) {
  function patchSettings(patch: Partial<AppSettings>) {
    onSettingsChange({ ...settings, ...patch });
  }

  return (
    <section className="settings-view">
      <div className="settings-header">
        <div>
          <h2>Settings</h2>
          <p>Local preferences for the StackDeck desktop interface.</p>
        </div>
        <button className="btn-refresh" disabled={refreshing} onClick={onRefresh} type="button">
          <IconRefresh size={14} />
          Refresh now
        </button>
      </div>

      <div className="settings-grid">
        <article className="settings-card">
          <div className="settings-card-title">
            <IconSun size={15} />
            <h3>Appearance</h3>
          </div>

          <label className="setting-row">
            <span>
              <strong>Theme</strong>
              <small>Choose the desktop color scheme.</small>
            </span>
            <select
              value={settings.theme}
              onChange={(event) => patchSettings({ theme: event.target.value as ThemePreference })}
            >
              <option value="system">System</option>
              <option value="dark">Dark</option>
              <option value="light">Light</option>
            </select>
          </label>

          <label className="setting-row">
            <span>
              <strong>Table density</strong>
              <small>Use tighter rows for large service lists.</small>
            </span>
            <input
              type="checkbox"
              role="switch"
              checked={settings.compactRows}
              onChange={(event) => patchSettings({ compactRows: event.target.checked })}
            />
          </label>

          <label className="setting-row">
            <span>
              <strong>System cards</strong>
              <small>Show CPU and RAM cards in the overview row.</small>
            </span>
            <input
              type="checkbox"
              role="switch"
              checked={settings.showSystemMetrics}
              onChange={(event) => patchSettings({ showSystemMetrics: event.target.checked })}
            />
          </label>
        </article>

        <article className="settings-card">
          <div className="settings-card-title">
            <IconRefresh size={15} />
            <h3>Refresh</h3>
          </div>

          <label className="setting-row">
            <span>
              <strong>Workspace auto-refresh</strong>
              <small>Periodically reload projects, services, and VM status.</small>
            </span>
            <select
              value={settings.autoRefreshSeconds}
              onChange={(event) => patchSettings({ autoRefreshSeconds: Number(event.target.value) })}
            >
              <option value={0}>Off</option>
              <option value={5}>Every 5 sec</option>
              <option value={10}>Every 10 sec</option>
              <option value={30}>Every 30 sec</option>
              <option value={60}>Every 1 min</option>
            </select>
          </label>

          <label className="setting-row">
            <span>
              <strong>System metric interval</strong>
              <small>Controls the CPU and RAM polling cadence.</small>
            </span>
            <select
              value={settings.statsRefreshSeconds}
              onChange={(event) => patchSettings({ statsRefreshSeconds: Number(event.target.value) })}
            >
              <option value={1}>1 sec</option>
              <option value={2}>2 sec</option>
              <option value={5}>5 sec</option>
              <option value={10}>10 sec</option>
            </select>
          </label>
        </article>

        <article className="settings-card">
          <div className="settings-card-title">
            <IconTerminal size={15} />
            <h3>Command Output</h3>
          </div>

          <label className="setting-row">
            <span>
              <strong>Open Logs on failure</strong>
              <small>Switch to Logs automatically when a command exits non-zero.</small>
            </span>
            <input
              type="checkbox"
              role="switch"
              checked={settings.openLogsOnFailure}
              onChange={(event) => patchSettings({ openLogsOnFailure: event.target.checked })}
            />
          </label>

          <label className="setting-row">
            <span>
              <strong>Open Logs for command output</strong>
              <small>Show Logs after successful Start, Stop, and Restart actions too.</small>
            </span>
            <input
              type="checkbox"
              role="switch"
              checked={settings.openLogsForCommandOutput}
              onChange={(event) => patchSettings({ openLogsForCommandOutput: event.target.checked })}
            />
          </label>
        </article>

        <article className="settings-card">
          <div className="settings-card-title">
            <IconLayers size={15} />
            <h3>Table Layout</h3>
          </div>

          <div className="setting-row">
            <span>
              <strong>Service columns</strong>
              <small>Restore the default column widths for the Services table.</small>
            </span>
            <button className="btn-ghost" onClick={onResetColumns} type="button">
              Reset columns
            </button>
          </div>
        </article>
      </div>
    </section>
  );
}

/* ─── App ──────────────────────────────────────────────────────────── */
function App() {
  const [projects, setProjects]               = useState<ProjectEntry[]>([]);
  const [selectedProject, setSelectedProject] = useState("");
  const [services, setServices]               = useState<ServiceEntry[]>([]);
  const [tab, setTab]                         = useState<Tab>("Services");
  const [overview, setOverview]               = useState<RuntimeOverview>(emptyOverview);
  const [logs, setLogs]                       = useState("");
  const [logContext, setLogContext]           = useState("");
  const [status, setStatus]                   = useState("Ready");
  const [refreshing, setRefreshing]           = useState(false);
  const isRefreshingRef                       = useRef(false);
  const [pendingAction, setPendingAction]     = useState("");
  const [lastUpdated, setLastUpdated]         = useState("");
  const [settings, setSettings]               = useState<AppSettings>(() => readAppSettings());
  const { stats: sysStats, history: sysHistory } = useSystemStats(settings.statsRefreshSeconds * 1000);

  const project = useMemo(
    () => projects.find((p) => p.name === selectedProject),
    [projects, selectedProject],
  );

  const endpoints = useMemo(
    () => services.reduce((t, s) => t + s.endpoints.length, 0),
    [services],
  );

  /* ── Data loaders ── */
  const loadProjects = useCallback(async () => {
    try {
      const items = await invoke<ProjectEntry[]>("read_projects");
      setProjects(items);
      setSelectedProject((cur) => {
        if (cur && items.some((p) => p.name === cur)) return cur;
        return items[0]?.name ?? "";
      });
      return items;
    } catch (err) {
      setProjects([]);
      setSelectedProject("");
      throw err;
    }
  }, []);

  const loadServices = useCallback(async (projectName: string) => {
    if (!projectName) { setServices([]); return []; }
    const items = await invoke<ServiceEntry[]>("list_services", { projectName });
    setServices(items);
    return items;
  }, []);

  const loadOverview = useCallback(async () => {
    const next = await invoke<RuntimeOverview>("hyperv_health");
    setOverview(next);
    return next;
  }, []);

  const refreshAll = useCallback(async () => {
    if (isRefreshingRef.current) return;
    isRefreshingRef.current = true;
    setRefreshing(true);
    setStatus("Refreshing workspace…");
    try {
      const items = await loadProjects();
      const name = selectedProject || items[0]?.name || "";
      await loadServices(name);
      await loadOverview();
      setLastUpdated(new Date().toLocaleTimeString());
      setStatus("Workspace refreshed");
    } catch (err) {
      setStatus(formatError(err));
    } finally {
      setRefreshing(false);
      isRefreshingRef.current = false;
    }
  }, [loadOverview, loadProjects, loadServices, selectedProject]);

  /* ── Side Effects ── */
  useEffect(() => { refreshAll(); }, []); // eslint-disable-line react-hooks/exhaustive-deps

  useEffect(() => {
    document.documentElement.dataset.theme = settings.theme;
    localStorage.setItem(appSettingsStorageKey, JSON.stringify(settings));
  }, [settings]);

  useEffect(() => {
    loadServices(selectedProject).catch((e) => setStatus(formatError(e)));
  }, [loadServices, selectedProject]);

  useEffect(() => {
    if (settings.autoRefreshSeconds <= 0) return;
    const id = window.setInterval(() => {
      refreshAll();
    }, settings.autoRefreshSeconds * 1000);
    return () => window.clearInterval(id);
  }, [refreshAll, settings.autoRefreshSeconds]);

  /* ── Actions ── */
  async function runServiceAction(action: string, serviceName?: string) {
    if (!selectedProject) { setStatus("Select a project first"); return; }

    const target = serviceName || "all services";
    const actionLabel = labelAction(action);
    const actionKey = `${selectedProject}:${action}:${target}`;
    setPendingAction(actionKey);
    setStatus(`${actionLabel} ${target}…`);
    setLogContext(`${actionLabel} — ${target}`);
    setLogs(formatCommandStart(actionLabel, selectedProject, target, project?.config || "-", project?.backend || "-"));

    try {
      const result = await invoke<CommandResult>("service_action", {
        projectName: selectedProject,
        action,
        serviceName,
      });
      const output = [
        formatCommandStart(actionLabel, selectedProject, target, project?.config || "-", project?.backend || "-"),
        `exit=${result.exit_code}${result.timed_out ? " timed_out=true" : ""}`,
        result.stdout && `stdout:\n${result.stdout}`,
        result.stderr && `stderr:\n${result.stderr}`,
      ].filter(Boolean).join("\n");
      setLogs(output || `exit=${result.exit_code}`);
      if (
        action === "logs" ||
        settings.openLogsForCommandOutput ||
        (settings.openLogsOnFailure && (result.exit_code !== 0 || result.timed_out))
      ) {
        setTab("Logs");
      }
      await loadServices(selectedProject);
      await loadOverview();
      setLastUpdated(new Date().toLocaleTimeString());
      setStatus(
        result.exit_code === 0
          ? `${actionLabel} finished`
          : `${actionLabel} failed (exit ${result.exit_code})`,
      );
    } catch (err) {
      const msg = formatError(err);
      setLogs(
        `${formatCommandStart(actionLabel, selectedProject, target, project?.config || "-", project?.backend || "-")}\nerror:\n${msg}`,
      );
      if (settings.openLogsOnFailure) {
        setTab("Logs");
      }
      setStatus(msg);
    } finally {
      setPendingAction("");
    }
  }

  async function clearVolumes() {
    if (!selectedProject) { setStatus("Select a project first"); return; }
    const target = "all services";
    const actionLabel = "Clear";
    const actionKey = `${selectedProject}:clear:${target}`;
    setPendingAction(actionKey);
    setStatus(`${actionLabel} ${target}…`);
    setLogContext(`${actionLabel} — ${target}`);
    setLogs(formatCommandStart(actionLabel, selectedProject, target, project?.config || "-", project?.backend || "-"));

    try {
      const result = await invoke<CommandResult>("clear_project_volumes", {
        projectName: selectedProject,
        confirmation: "CLEAR_VOLUMES",
      });
      const output = [
        formatCommandStart(actionLabel, selectedProject, target, project?.config || "-", project?.backend || "-"),
        `exit=${result.exit_code}${result.timed_out ? " timed_out=true" : ""}`,
        result.stdout && `stdout:\n${result.stdout}`,
        result.stderr && `stderr:\n${result.stderr}`,
      ].filter(Boolean).join("\n");
      setLogs(output || `exit=${result.exit_code}`);
      await loadServices(selectedProject);
      await loadOverview();
      setLastUpdated(new Date().toLocaleTimeString());
      setStatus(result.exit_code === 0 ? "Clear finished" : `Clear failed (exit ${result.exit_code})`);
    } catch (err) {
      const msg = formatError(err);
      setLogs(`${formatCommandStart(actionLabel, selectedProject, target, project?.config || "-", project?.backend || "-")}\nerror:\n${msg}`);
      setStatus(msg);
    } finally {
      setPendingAction("");
    }
  }

  async function unregisterProject() {
      if (!selectedProject) { setStatus("Select a project first"); return; }
      const actionLabel = "Unregister";
      const actionKey = `${selectedProject}:unregister`;
      setPendingAction(actionKey);
      setStatus(`${actionLabel} project…`);
      setLogContext(`${actionLabel} — project`);
      setLogs(`Unregistering ${selectedProject}...`);

      try {
        const result = await invoke<CommandResult>("unregister_project", {
          projectName: selectedProject,
          confirmation: "UNREGISTER",
        });
        const output = [
          `exit=${result.exit_code}${result.timed_out ? " timed_out=true" : ""}`,
          result.stdout && `stdout:\n${result.stdout}`,
          result.stderr && `stderr:\n${result.stderr}`,
        ].filter(Boolean).join("\n");
        setLogs(output || `exit=${result.exit_code}`);

        if (result.exit_code === 0) {
          setSelectedProject("");
          await loadProjects();
          setStatus("Project successfully removed.");
        } else {
          setStatus(`Remove failed (exit ${result.exit_code})`);
        }
      } catch (err) {
        const msg = formatError(err);
        setLogs(`error:\n${msg}`);
        setStatus(msg);
      } finally {
        setPendingAction("");
      }
    }

  async function openEndpoint(serviceName: string, hostPort: number) {
    if (!selectedProject) { setStatus("Select a project first"); return; }
    try {
      const msg = await invoke<string>("open_service_url", {
        projectName: selectedProject,
        serviceName,
        hostPort,
      });
      setStatus(msg);
    } catch (err) {
      setStatus(formatError(err));
    }
  }

  /* ── Render ── */
  return (
    <main className="app-shell" data-density={settings.compactRows ? "compact" : "comfortable"}>
      {/* ── Sidebar ── */}
      <aside className="sidebar">
        <div className="brand">
          <div className="brand-logo">
            <BrandGlyph />
          </div>
          <div className="brand-text">
            <h1>StackDeck</h1>
            <p>Desktop control plane</p>
          </div>
        </div>

        <nav className="tabs" aria-label="Primary navigation">
          {tabs.map((item) => (
            <button
              key={item}
              className={`tab-item${tab === item ? " active" : ""}`}
              onClick={() => setTab(item)}
              type="button"
            >
              <span className="tab-icon">{tabIcons[item]}</span>
              <span className="tab-label">{item}</span>
              {item === "Services" && services.length > 0 && (
                <span className="tab-badge">{services.length}</span>
              )}
            </button>
          ))}
        </nav>

        <div className="sidebar-footer">
          <div className="theme-select-wrap">
            <IconSun size={14} />
            <select
              id="theme"
              value={settings.theme}
              onChange={(e) => setSettings((current) => ({ ...current, theme: e.target.value as ThemePreference }))}
              aria-label="Theme preference"
            >
              <option value="system">System theme</option>
              <option value="dark">Dark</option>
              <option value="light">Light</option>
            </select>
          </div>
        </div>
      </aside>

      {/* ── Main Workspace ── */}
      <section className="workspace">
        {/* Toolbar */}
        <header className="toolbar">
          <div className="project-picker">
            <label htmlFor="project">Project</label>
            <select
              id="project"
              value={selectedProject}
              onChange={(e) => setSelectedProject(e.target.value)}
            >
              {projects.length === 0 && <option value="">No registered projects</option>}
              {projects.map((p) => (
                <option key={p.name} value={p.name}>{p.name}</option>
              ))}
            </select>
          </div>

          <div className="meta-strip">
            <MetaPill label="Backend" value={project?.backend || "—"} />
            <MetaPill label="Config"  value={project?.config  || "—"} />
            <MetaPill label="Root"    value={project?.root    || "—"} />
          </div>

          <div className="toolbar-actions">
            <button
              className="btn-start-all"
              disabled={Boolean(pendingAction) || !selectedProject}
              onClick={() => runServiceAction("up")}
              title="Start all services"
            >
              <IconPlay size={12} />
              Start all
            </button>
            <button
              className="btn-stop-all"
              disabled={Boolean(pendingAction) || !selectedProject}
              onClick={() => runServiceAction("down")}
              title="Stop all services"
            >
              <IconStop size={11} />
              Stop all
            </button>
            <button
              className="btn-clear-all"
              disabled={Boolean(pendingAction) || !selectedProject}
              onClick={() => {
                if (window.confirm("Are you sure you want to stop all services and remove their volumes? Data might be lost!")) {
                  clearVolumes();
                }
              }}
              title="Stop and clear all services (removes volumes)"
            >
              <IconTrash size={12} />
              Clear all
            </button>
            <button
              className="btn-clear-all"
              disabled={Boolean(pendingAction) || !selectedProject}
              onClick={() => {
                if (window.confirm("Are you sure you want to completely remove this project from StackDeck? (This also runs Clear All internally)")) {
                  unregisterProject();
                }
              }}
              title="Remove this project from StackDeck entirely"
              style={{ backgroundColor: '#9b2c2c', borderColor: '#742a2a', color: 'white' }}
            >
              <IconTrash size={12} />
              Remove Project
            </button>
            <button
              className={`btn-refresh${refreshing ? " spinning" : ""}`}
              disabled={refreshing}
              onClick={refreshAll}
              title="Refresh"
            >
              <IconRefresh size={14} />
              {refreshing ? "Refreshing…" : "Refresh"}
            </button>
          </div>
        </header>

        {/* Summary Row */}
        <section className="summary-grid" aria-label="Overview metrics">
          <MetricCard label="Projects"    value={projects.length} />
          <MetricCard label="Services"    value={services.length} />
          <MetricCard label="Endpoints"   value={endpoints} />
          <MetricCard label="Last update" value={lastUpdated || "Not yet"} text />
          {settings.showSystemMetrics && <CpuCard stats={sysStats} history={sysHistory.cpu} />}
          {settings.showSystemMetrics && <RamCard stats={sysStats} history={sysHistory.ram} />}
        </section>

        {/* Content */}
        <div className="content">
          {tab === "Services"   && (
            <ServicesView
              services={services}
              pendingAction={pendingAction}
              onAction={runServiceAction}
              onOpen={openEndpoint}
            />
          )}
          {tab === "Containers" && <TextPanel title="Containers"    text={overview.containers} onBack={() => setTab("Services")} />}
          {tab === "Images"     && <TextPanel title="Images"        text={overview.images} onBack={() => setTab("Services")} />}
          {tab === "Volumes"    && <TextPanel title="Volumes"       text={overview.volumes} onBack={() => setTab("Services")} />}
          {tab === "Networks"   && <TextPanel title="Networks"      text={overview.networks} onBack={() => setTab("Services")} />}
          {tab === "VM"         && <TextPanel title="Hyper-V health" text={overview.vm_health} onBack={() => setTab("Services")} />}
          {tab === "Logs"       && <TextPanel title={logContext || "Command output"} text={logs} onBack={() => setTab("Services")} />}
          {tab === "Settings"   && (
            <SettingsView
              settings={settings}
              onSettingsChange={setSettings}
              onResetColumns={() => {
                resetServiceColumnWidths();
                setStatus("Service columns reset");
              }}
              onRefresh={refreshAll}
              refreshing={refreshing}
            />
          )}
        </div>

        {/* Status Bar */}
        <footer className="statusbar">
          <span className={`state-badge${pendingAction || refreshing ? " busy" : ""}`}>
            <span className="state-dot" />
            {pendingAction ? "Running" : refreshing ? "Refreshing" : "Idle"}
          </span>
          <span className="statusbar-message">{status}</span>
        </footer>
      </section>
    </main>
  );
}

/* ─── Mount ─────────────────────────────────────────────────────────── */
createRoot(document.getElementById("root") as HTMLElement).render(
  <React.StrictMode>
    <App />
  </React.StrictMode>,
);
