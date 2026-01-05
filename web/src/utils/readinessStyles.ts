import type React from 'react';

export interface ReadinessStyles {
  readinessValue: number;
  clampedReadiness: number;
  ringDeg: number;
  isCriticalReadiness: boolean;
  isWarningReadiness: boolean;
  isPerfectReadiness: boolean;
  ringPrimary: string;
  ringSecondary: string;
  readinessTextClass: string;
  readinessLabelClass: string;
  readinessGlowColor: string;
  ringBackground: string;
  miniRingStyle: React.CSSProperties;
}

export function computeReadinessStyles(readiness: number | null): ReadinessStyles {
  const readinessValue = readiness ?? 0;
  const clampedReadiness = Math.max(0, Math.min(100, readinessValue));
  const ringDeg = Math.round((clampedReadiness / 100) * 360);
  const isCriticalReadiness = clampedReadiness === 0;
  const isWarningReadiness = !isCriticalReadiness && clampedReadiness <= 33;
  const isPerfectReadiness = clampedReadiness === 100;

  const ringPrimary = isCriticalReadiness
    ? 'rgba(239, 68, 68, 0.86)'
    : isWarningReadiness
      ? 'rgba(245, 158, 11, 0.86)'
      : isPerfectReadiness
        ? 'rgba(52, 211, 153, 0.92)'
        : 'rgba(56, 189, 248, 0.82)';

  const ringSecondary = isCriticalReadiness
    ? 'rgba(248, 113, 113, 0.78)'
    : isWarningReadiness
      ? 'rgba(251, 191, 36, 0.78)'
      : isPerfectReadiness
        ? 'rgba(56, 189, 248, 0.88)'
        : 'rgba(52, 211, 153, 0.78)';

  const readinessTextClass = isCriticalReadiness
    ? 'text-error'
    : isWarningReadiness
      ? 'text-warning'
      : 'text-on-surface';

  const readinessLabelClass = isCriticalReadiness
    ? 'text-error-light/80'
    : isWarningReadiness
      ? 'text-warning/80'
      : isPerfectReadiness
        ? 'text-on-surface/80'
        : 'text-on-surface-variant';

  const readinessGlowColor = isCriticalReadiness
    ? 'rgba(239, 68, 68, 0.25)'
    : isWarningReadiness
      ? 'rgba(245, 158, 11, 0.2)'
      : isPerfectReadiness
        ? 'rgba(52, 211, 153, 0.35)'
        : 'rgba(56, 189, 248, 0.2)';

  const ringBackground = isPerfectReadiness
    ? `conic-gradient(${ringPrimary} 0deg, ${ringSecondary} 120deg, rgba(192, 132, 252, 0.8) 240deg, ${ringPrimary} 360deg)`
    : `conic-gradient(${ringPrimary} 0deg, ${ringSecondary} ${ringDeg}deg, rgba(79, 99, 127, 0.22) ${ringDeg}deg 360deg)`;

  const miniRingStyle: React.CSSProperties = {
    background: ringBackground,
    WebkitMaskImage: 'radial-gradient(farthest-side, transparent calc(100% - 2.5px), #000 calc(100% - 2.5px))',
    maskImage: 'radial-gradient(farthest-side, transparent calc(100% - 2.5px), #000 calc(100% - 2.5px))',
  };

  return {
    readinessValue,
    clampedReadiness,
    ringDeg,
    isCriticalReadiness,
    isWarningReadiness,
    isPerfectReadiness,
    ringPrimary,
    ringSecondary,
    readinessTextClass,
    readinessLabelClass,
    readinessGlowColor,
    ringBackground,
    miniRingStyle,
  };
}
