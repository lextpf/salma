import type { ReactNode } from 'react'
import Kicker from './Kicker'

interface ModuleHeaderProps {
  num: string
  label: string
  children?: ReactNode
}

/**
 * The 52px bar every module opens with.
 *
 * One component for all four pages, so the height and the padding stay in
 * step. It carries no fill and no rule of its own, so it sits on the same plane
 * as the module below it. Pages pass their controls as children and own the
 * flex spacer between the left-hand group and the right-hand actions.
 */
export default function ModuleHeader({ num, label, children }: ModuleHeaderProps) {
  return (
    <div
      style={{
        height: 52,
        flexShrink: 0,
        display: 'flex',
        alignItems: 'center',
        gap: 13,
        padding: '0 18px',
        position: 'relative',
        zIndex: 20,
      }}
    >
      <Kicker num={num} label={label} />
      {children}
    </div>
  )
}
