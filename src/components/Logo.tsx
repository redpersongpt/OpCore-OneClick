import { useId } from 'react';

export default function Logo({ size = 32, className = '' }: { size?: number; className?: string }) {
  const uid = useId().replace(/:/g, '');
  const ring = `oc-ring-${uid}`;
  const core = `oc-core-${uid}`;
  const glow = `oc-glow-${uid}`;

  return (
    <svg width={size} height={size} viewBox="0 0 100 100" fill="none" className={className} aria-hidden>
      <defs>
        <linearGradient id={ring} x1="10" y1="10" x2="90" y2="90" gradientUnits="userSpaceOnUse">
          <stop stopColor="currentColor" stopOpacity="0.92" />
          <stop offset="1" stopColor="currentColor" stopOpacity="0.3" />
        </linearGradient>
        <linearGradient id={core} x1="32" y1="24" x2="71" y2="76" gradientUnits="userSpaceOnUse">
          <stop stopColor="#f8fafc" />
          <stop offset="0.52" stopColor="#d9dde7" />
          <stop offset="1" stopColor="#8e97ab" />
        </linearGradient>
        <radialGradient
          id={glow}
          cx="0"
          cy="0"
          r="1"
          gradientUnits="userSpaceOnUse"
          gradientTransform="translate(74 22) rotate(135) scale(20)"
        >
          <stop stopColor="#9bd1ff" />
          <stop offset="1" stopColor="#3b82f6" stopOpacity="0" />
        </radialGradient>
      </defs>
      <circle cx="50" cy="50" r="45" stroke={`url(#${ring})`} strokeWidth="2" strokeDasharray="7 9" />
      <circle cx="50" cy="50" r="34" stroke="currentColor" strokeOpacity="0.18" strokeWidth="1.5" />
      <path
        d="M50 17L73 28V54C73 67 63.8 77.6 50 83C36.2 77.6 27 67 27 54V28L50 17Z"
        fill={`url(#${core})`}
        fillOpacity="0.08"
        stroke="currentColor"
        strokeOpacity="0.12"
        strokeWidth="1"
      />
      <path
        d="M63 33.5C58.9 29.9 54 28 48.7 28C35.2 28 25.2 38 25.2 50.5C25.2 63 35.2 73 48.7 73C54 73 58.9 71.1 63 67.5"
        stroke={`url(#${core})`}
        strokeWidth="9.5"
        strokeLinecap="round"
      />
      <path d="M58 34L43 67" stroke="#f0f4ff" strokeWidth="7" strokeLinecap="round" />
      <circle cx="74" cy="22" r="11" fill={`url(#${glow})`} />
      <circle cx="74" cy="22" r="4.5" fill="#4ea6ff" />
      <path d="M72 33L63.5 41" stroke="#4ea6ff" strokeOpacity="0.75" strokeWidth="2" strokeLinecap="round" />
    </svg>
  );
}
