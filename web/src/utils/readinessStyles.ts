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
}

/**
 * Circular ring mask with a 1px feathered inner edge.
 *
 * Spreading the stops +/-0.5px around the target thickness gives the
 * compositor a 1px band to feather the inner circle cleanly.
 */
export function ringMaskStyle(thicknessPx: number): React.CSSProperties {
  const outer = thicknessPx + 0.5;
  const inner = Math.max(0.1, thicknessPx - 0.5);
  const mask = `radial-gradient(farthest-side, transparent calc(100% - ${outer}px), #000 calc(100% - ${inner}px))`;
  return {
    WebkitMaskImage: mask,
    maskImage: mask,
  };
}

export function computeReadinessStyles(readiness: number | null): ReadinessStyles {
  const readinessValue = readiness ?? 0;
  const clampedReadiness = Math.max(0, Math.min(100, readinessValue));
  const ringDeg = Math.round((clampedReadiness / 100) * 360);
  const isCriticalReadiness = clampedReadiness === 0;
  const isWarningReadiness = !isCriticalReadiness && clampedReadiness <= 33;
  const isPerfectReadiness = clampedReadiness === 100;

  // Single-tone Atelier ring: oxblood (critical/error), ochre (warning), moss (perfect/ok), ink-3 default.
  const ringPrimary = isCriticalReadiness
    ? 'var(--accent)'
    : isWarningReadiness
      ? 'var(--ochre)'
      : isPerfectReadiness
        ? 'var(--moss)'
        : 'var(--moss)';

  const ringSecondary = ringPrimary;

  const readinessTextClass = isCriticalReadiness
    ? 'text-error'
    : isWarningReadiness
      ? 'text-warning'
      : 'text-on-surface';

  const readinessLabelClass = isCriticalReadiness
    ? 'text-error/80'
    : isWarningReadiness
      ? 'text-warning/80'
      : 'text-on-surface-variant';

  const readinessGlowColor = isCriticalReadiness
    ? 'rgba(138, 42, 31, 0.20)'
    : isWarningReadiness
      ? 'rgba(166, 122, 42, 0.20)'
      : 'rgba(61, 122, 61, 0.20)';

  const trackColor = 'var(--readiness-ring-track, rgba(154, 146, 127, 0.30))';
  const ringBackground = isPerfectReadiness
    ? `conic-gradient(${ringPrimary} 0deg, ${ringPrimary} 360deg)`
    : `conic-gradient(${ringPrimary} 0deg, ${ringPrimary} ${ringDeg}deg, ${trackColor} ${ringDeg}deg 360deg)`;

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
  };
}
