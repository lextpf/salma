import { Component, ReactNode } from 'react'

interface Props {
  children: ReactNode
}

interface State {
  hasError: boolean
  error: Error | null
}

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
          className="flex flex-col items-center justify-center"
          style={{ minHeight: '60vh', gap: 16, padding: 24 }}
        >
          <h2
            className="display-serif-tight"
            style={{ fontSize: 32, color: 'var(--ink)' }}
          >
            Something went wrong<span className="display-period">.</span>
          </h2>
          <p
            className="timestamp-print"
            style={{ fontSize: 12, textAlign: 'center', maxWidth: 520 }}
          >
            // {this.state.error?.message || 'An unexpected error occurred'}
          </p>
          <button
            type="button"
            className="tool-btn tool-btn-ink"
            onClick={() => this.setState({ hasError: false, error: null })}
          >
            <i className="fa-duotone fa-solid fa-rotate-right" style={{ fontSize: 12 }} />
            <span>Try again</span>
          </button>
        </div>
      )
    }
    return this.props.children
  }
}
