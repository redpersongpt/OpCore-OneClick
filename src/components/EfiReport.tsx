// ── EFI Intelligence Report ──────────────────────────────────────────────────
// Premium panel that explains every decision made during EFI generation.

import React, { useState } from 'react';
import {
  ChevronDown, ChevronRight, Shield, Cpu, HardDrive, Zap, AlertTriangle,
  CheckCircle, Info, Lock, Wifi, Monitor, Terminal,
} from 'lucide-react';
import type { EfiReport, KextExplanation, BootArgExplanation, KnownLimitation } from '../lib/efiReport';
import type { ConfidenceFactor } from '../lib/confidenceScore';

interface Props {
  report: EfiReport;
}

// ── Confidence ring ─────────────────────────────────────────────────────────

function ConfidenceRing({ score }: { score: number }) {
  const circumference = 2 * Math.PI * 40;
  const offset = circumference - (score / 100) * circumference;
  const color = score >= 75 ? '#34d399' : score >= 50 ? '#fbbf24' : score >= 25 ? '#f97316' : '#ef4444';
  const glowColor = score >= 75 ? 'rgba(52,211,153,0.15)' : score >= 50 ? 'rgba(251,191,36,0.15)' : score >= 25 ? 'rgba(249,115,22,0.15)' : 'rgba(239,68,68,0.15)';
  const grade = score >= 85 ? 'Excellent' : score >= 70 ? 'Good' : score >= 50 ? 'Fair' : score >= 25 ? 'Risky' : 'Critical';

  return (
    <div className="relative w-24 h-24 flex-shrink-0">
      <div className="absolute inset-0 rounded-full blur-xl" style={{ backgroundColor: glowColor }} />
      <svg className="w-24 h-24 -rotate-90 relative z-10" viewBox="0 0 96 96">
        <circle cx="48" cy="48" r="40" fill="none" stroke="rgba(255,255,255,0.05)" strokeWidth="6" />
        <circle
          cx="48" cy="48" r="40" fill="none" stroke={color} strokeWidth="6"
          strokeLinecap="round"
          strokeDasharray={circumference}
          strokeDashoffset={offset}
          style={{ transition: 'stroke-dashoffset 1s ease-out' }}
        />
      </svg>
      <div className="absolute inset-0 flex flex-col items-center justify-center z-10">
        <span className="text-2xl font-black text-white">{score}</span>
        <span className="text-[7px] font-bold uppercase tracking-widest" style={{ color }}>{grade}</span>
      </div>
    </div>
  );
}

// ── Section wrapper ─────────────────────────────────────────────────────────

function Section({ title, icon: Icon, children, defaultOpen = false }: {
  title: string;
  icon: React.ElementType;
  children: React.ReactNode;
  defaultOpen?: boolean;
}) {
  const [open, setOpen] = useState(defaultOpen);
  return (
    <div className="rounded-2xl border border-white/6 overflow-hidden">
      <button
        onClick={() => setOpen(!open)}
        className="w-full flex items-center gap-3 px-5 py-3.5 bg-white/[0.02] hover:bg-white/[0.04] transition-colors cursor-pointer"
      >
        <Icon className="w-4 h-4 text-white/30" />
        <span className="text-xs font-bold text-white/60 uppercase tracking-widest flex-1 text-left">{title}</span>
        {open ? <ChevronDown className="w-4 h-4 text-white/20" /> : <ChevronRight className="w-4 h-4 text-white/20" />}
      </button>
      {open && <div className="border-t border-white/5">{children}</div>}
    </div>
  );
}

// ── Kext row ────────────────────────────────────────────────────────────────

function KextRow({ kext }: { kext: KextExplanation; key?: string }) {
  const [expanded, setExpanded] = useState(false);
  const catColor: Record<string, string> = {
    'must-have': 'text-blue-400 bg-blue-500/10 border-blue-500/20',
    gpu: 'text-purple-400 bg-purple-500/10 border-purple-500/20',
    audio: 'text-emerald-400 bg-emerald-500/10 border-emerald-500/20',
    ethernet: 'text-cyan-400 bg-cyan-500/10 border-cyan-500/20',
    wifi: 'text-sky-400 bg-sky-500/10 border-sky-500/20',
    usb: 'text-orange-400 bg-orange-500/10 border-orange-500/20',
    laptop: 'text-pink-400 bg-pink-500/10 border-pink-500/20',
    extras: 'text-white/40 bg-white/5 border-white/10',
    amd: 'text-red-400 bg-red-500/10 border-red-500/20',
  };

  return (
    <div className="border-b border-white/[0.03] last:border-0">
      <button
        onClick={() => setExpanded(!expanded)}
        className="w-full flex items-center gap-3 px-5 py-3 hover:bg-white/[0.02] transition-colors cursor-pointer text-left"
      >
        <div className="flex-1 min-w-0">
          <div className="flex items-center gap-2">
            <span className="text-sm font-bold text-white/80">{kext.name}</span>
            {kext.version && <span className="text-[10px] text-white/25 font-mono">v{kext.version}</span>}
          </div>
        </div>
        <span className={`px-2 py-0.5 rounded-md text-[9px] font-bold uppercase border ${catColor[kext.category] ?? catColor.extras}`}>
          {kext.category}
        </span>
        {expanded ? <ChevronDown className="w-3.5 h-3.5 text-white/15" /> : <ChevronRight className="w-3.5 h-3.5 text-white/15" />}
      </button>
      {expanded && (
        <div className="px-5 pb-3 space-y-2">
          <p className="text-xs text-white/45 leading-relaxed">{kext.reason}</p>
          {kext.dependencies.length > 0 && (
            <p className="text-[10px] text-white/25">Depends on: {kext.dependencies.join(', ')}</p>
          )}
          <p className="text-[10px] text-white/15">Canonicality: {kext.canonicality}</p>
        </div>
      )}
    </div>
  );
}

// ── Boot arg row ────────────────────────────────────────────────────────────

function BootArgRow({ arg }: { arg: BootArgExplanation; key?: number }) {
  const impactColor = arg.impact === 'critical' ? 'text-red-400' : arg.impact === 'functional' ? 'text-amber-400' : 'text-white/30';
  return (
    <div className="flex items-start gap-3 px-5 py-2.5 border-b border-white/[0.03] last:border-0">
      <code className="text-xs font-mono text-blue-300/70 bg-blue-500/8 px-2 py-0.5 rounded flex-shrink-0">{arg.arg}</code>
      <div className="flex-1 min-w-0">
        <p className="text-xs text-white/45 leading-relaxed">{arg.purpose}</p>
      </div>
      <span className={`text-[9px] font-bold uppercase ${impactColor} flex-shrink-0`}>{arg.impact}</span>
    </div>
  );
}

// ── Limitation row ──────────────────────────────────────────────────────────

function LimitationRow({ limitation }: { limitation: KnownLimitation; key?: number }) {
  const [expanded, setExpanded] = useState(false);
  const sevIcon = limitation.severity === 'high'
    ? <AlertTriangle className="w-3.5 h-3.5 text-red-400" />
    : limitation.severity === 'medium'
    ? <AlertTriangle className="w-3.5 h-3.5 text-amber-400" />
    : <Info className="w-3.5 h-3.5 text-white/30" />;

  return (
    <div className="border-b border-white/[0.03] last:border-0">
      <button
        onClick={() => setExpanded(!expanded)}
        className="w-full flex items-center gap-3 px-5 py-3 hover:bg-white/[0.02] transition-colors cursor-pointer text-left"
      >
        {sevIcon}
        <div className="flex-1 min-w-0">
          <span className="text-sm font-semibold text-white/70">{limitation.area}</span>
          <p className="text-xs text-white/35 leading-relaxed mt-0.5">{limitation.description}</p>
        </div>
        {limitation.workaround && (
          expanded ? <ChevronDown className="w-3.5 h-3.5 text-white/15" /> : <ChevronRight className="w-3.5 h-3.5 text-white/15" />
        )}
      </button>
      {expanded && limitation.workaround && (
        <div className="px-5 pb-3 ml-7">
          <div className="flex items-start gap-2 px-3 py-2 rounded-lg bg-emerald-500/5 border border-emerald-500/10">
            <CheckCircle className="w-3.5 h-3.5 text-emerald-400 flex-shrink-0 mt-0.5" />
            <p className="text-xs text-emerald-300/60 leading-relaxed">{limitation.workaround}</p>
          </div>
        </div>
      )}
    </div>
  );
}

// ── Main component ──────────────────────────────────────────────────────────

export default function EfiReportPanel({ report }: Props) {
  return (
    <div className="space-y-4">
      {/* ── Confidence header ──────────────────────────────────────── */}
      <div className="flex items-center gap-6 p-6 rounded-2xl bg-white/[0.02] border border-white/6 relative overflow-hidden">
        <ConfidenceRing score={report.confidenceScore} />
        <div className="flex-1 min-w-0">
          <div className="text-[9px] font-bold text-white/20 uppercase tracking-widest mb-1">Build Assessment</div>
          <h3 className="text-lg font-bold text-white mb-1.5">{report.confidenceExplanation}</h3>
          <p className="text-xs text-white/35 leading-relaxed">
            Based on CPU maturity, GPU driver support, kext complexity, and known hardware quirks.
          </p>
          {report.macOSCeiling && (
            <div className="mt-2.5 flex items-center gap-2 px-2.5 py-1.5 rounded-lg bg-amber-500/8 border border-amber-500/15 w-fit">
              <AlertTriangle className="w-3 h-3 text-amber-400" />
              <span className="text-[10px] text-amber-400/80 font-medium">Max supported: {report.macOSCeiling.version}</span>
            </div>
          )}
        </div>
      </div>

      {/* ── Hardware summary ───────────────────────────────────────── */}
      <Section title="Hardware Summary" icon={Cpu} defaultOpen>
        <div className="divide-y divide-white/[0.03]">
          {report.hardware.items.map(item => (
            <div key={item.label} className="flex items-center justify-between px-5 py-2.5">
              <span className="text-xs text-white/35 font-medium w-28 flex-shrink-0">{item.label}</span>
              <div className="flex-1 text-right">
                <span className="text-sm text-white/70 font-medium">{item.value}</span>
                {item.detail && <div className="text-[10px] text-white/25">{item.detail}</div>}
              </div>
            </div>
          ))}
        </div>
      </Section>

      {/* ── SMBIOS ─────────────────────────────────────────────────── */}
      <Section title="SMBIOS Selection" icon={Monitor}>
        <div className="p-5 space-y-3">
          <div className="flex items-center gap-3">
            <code className="text-base font-mono font-bold text-white bg-white/5 px-4 py-1.5 rounded-lg border border-white/8">{report.smbios.selected}</code>
            <span className="text-[9px] text-white/20 font-bold uppercase tracking-widest">Mac identity</span>
          </div>
          <p className="text-xs text-white/45 leading-relaxed">{report.smbios.reasoning}</p>
          {report.smbios.alternatives.length > 0 && (
            <div className="space-y-1.5 pt-2 border-t border-white/5">
              <span className="text-[10px] font-bold text-white/20 uppercase tracking-widest">Alternatives</span>
              {report.smbios.alternatives.map((alt, i) => (
                <p key={i} className="text-[11px] text-white/30 leading-relaxed">• {alt}</p>
              ))}
            </div>
          )}
        </div>
      </Section>

      {/* ── Kexts ──────────────────────────────────────────────────── */}
      <Section title={`Kernel Extensions (${report.kexts.length})`} icon={Zap}>
        {report.kexts.map(k => <KextRow key={k.name} kext={k} />)}
      </Section>

      {/* ── Boot args ──────────────────────────────────────────────── */}
      <Section title={`Boot Arguments (${report.bootArgs.length})`} icon={Terminal}>
        {report.bootArgs.map((a, i) => <BootArgRow key={i} arg={a} />)}
      </Section>

      {/* ── Known limitations ──────────────────────────────────────── */}
      {report.limitations.length > 0 && (
        <Section title={`Known Limitations (${report.limitations.length})`} icon={AlertTriangle}>
          {report.limitations.map((l, i) => <LimitationRow key={i} limitation={l} />)}
        </Section>
      )}
    </div>
  );
}
