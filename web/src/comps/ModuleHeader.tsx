import type { ReactNode } from 'react'
import Kicker from './Kicker'

interface ModuleHeaderProps {
  num: string
  label: string
  children?: ReactNode
}

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
