import { Component, ReactNode } from 'react'
import Button from './Button'

interface Props {
  children: ReactNode
}

interface State {
  hasError: boolean
  error: Error | null
}

/**
 * Last-resort fault screen.
 *
 * Stacked bands on the bare plane rather than a centred card: a legend, the
 * headline, the raw message, then the recovery control beside a line of
 * metadata. Nothing is boxed and nothing is painted, so the fault reads as part
 * of the instrument instead of a modal dropped on top of it.
 *
 * Catches render-phase errors only. An error thrown in an event handler, in a
 * promise or in a polling hook never reaches here; the page that owns it
 * reports it instead.
 */
export default class ErrorBoundary extends Component<Props, State> {
  state: State = { hasError: false, error: null }

  static getDerivedStateFromError(error: Error): State {
    return { hasError: true, error }
  }

  componentDidCatch(error: Error, info: React.ErrorInfo) {
    console.error('[ErrorBoundary]', error, info.componentStack)
  }

  render() {
    if (this.state.hasError) {
      return (
        <div
          className="flex items-center justify-center"
          style={{ minHeight: '60vh', padding: 24 }}
        >
          <div style={{ width: '100%', maxWidth: 560 }}>
            {/* Section label, not a badge: the same tracked mono legend every
                other band in the app opens with. */}
            <div
              className="flex items-center"
              style={{
                gap: 10,
                fontFamily: 'var(--font-mono)',
                fontSize: 'var(--fs-micro)',
                fontWeight: 600,
                textTransform: 'uppercase',
                letterSpacing: 'var(--tr-kicker)',
                color: 'var(--danger)',
              }}
            >
              <span aria-hidden="true" style={{ width: 4, height: 4, background: 'var(--danger)' }} />
              <span>Render fault</span>
            </div>
            <h2
              style={{
                marginTop: 10,
                fontSize: 'var(--fs-empty)',
                fontWeight: 700,
                letterSpacing: 'var(--tr-tight)',
                color: 'var(--ink)',
              }}
            >
              Something went wrong<span style={{ color: 'var(--signal-2)' }}>.</span>
            </h2>
            <p
              style={{
                margin: 0,
                marginTop: 10,
                paddingTop: 10,
                fontFamily: 'var(--font-mono)',
                fontSize: 'var(--fs-mono)',
                lineHeight: 'var(--lh-body)',
                color: 'var(--ink-3)',
                textWrap: 'pretty',
              }}
            >
              // {this.state.error?.message || 'An unexpected error occurred'}
            </p>
            <div
              className="flex items-center"
              style={{
                gap: 12,
                flexWrap: 'wrap',
                marginTop: 12,
                paddingTop: 12,
              }}
            >
              <Button
                icon="refresh"
                label="Try again"
                variant="primary"
                onClick={() => { this.setState({ hasError: false, error: null }); }}
              />
              <span
                style={{
                  fontFamily: 'var(--font-mono)',
                  fontSize: 'var(--fs-meta)',
                  // The only instruction on the screen, so it holds a text
                  // ink level rather than the meta floor.
                  color: 'var(--ink-5)',
                }}
              >
                view unmounted // full trace in the browser console
              </span>
            </div>
          </div>
        </div>
      )
    }
    return this.props.children
  }
}
