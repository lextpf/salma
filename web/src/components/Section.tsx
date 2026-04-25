import type { ReactNode } from 'react'

interface SectionProps {
  /** Italic mono numeral shown in the header bar (e.g. "01", "02", "ii") */
  n: string
  /** Cap-styled section label following the § glyph (e.g. "Upload", "Activity") */
  label: string
  /** Serif title shown alongside the label (e.g. "Select an archive") */
  title: string
  /** Big faded numeral floated in the section's top-right corner (usually equals `n`) */
  corner: string
  /** Right-side header content — badges, action buttons, "Full log →" link */
  meta?: ReactNode
  /** When 'none', body has no padding (caller paints its own — used for tables/queues) */
  bodyPadding?: 'none' | 'normal'
  /** Extra utility classes for the outer section element */
  className?: string
  children: ReactNode
}

export default function Section({
  n,
  label,
  title,
  corner,
  meta,
  bodyPadding = 'normal',
  className = '',
  children,
}: SectionProps) {
  return (
    <section className={`atelier-section ${className}`.trim()}>
      <div className="atelier-section-header">
        <div className="atelier-section-header-lead">
          <span className="atelier-section-num">{n}</span>
          <span className="atelier-section-dash" aria-hidden="true" />
          <span className="chapter-label" style={{ gap: '0.4rem' }}>
            <span>§ {label}</span>
          </span>
          <span className="atelier-section-title">{title}</span>
        </div>
        {meta ? <div className="atelier-section-meta">{meta}</div> : null}
      </div>
      <span aria-hidden="true" className="atelier-section-corner">
        {corner}
      </span>
      <div
        className={`atelier-section-body${bodyPadding === 'normal' ? ' atelier-section-body-pad' : ''}`}
      >
        {children}
      </div>
    </section>
  )
}
