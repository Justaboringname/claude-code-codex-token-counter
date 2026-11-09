// Render-error boundary. A snapshot from another Mac (or a cache from an older
// build) is validated before it is shown, but a render bug must still never
// blank the whole window: the boundary swaps the failing part for a fallback
// that keeps recovery in reach (Unpair, clear the cached copy, This Mac).
import { Component, type ErrorInfo, type ReactNode } from 'react';

interface Props {
  /** Names the boundary in the console message. */
  label: string;
  /** A new value (other range, new snapshot version, other source) retries. */
  resetKey?: string;
  fallback: (reset: () => void) => ReactNode;
  children: ReactNode;
}

interface State {
  failed: boolean;
}

export class ErrorBoundary extends Component<Props, State> {
  state: State = { failed: false };

  static getDerivedStateFromError(): State {
    return { failed: true };
  }

  componentDidCatch(error: Error, info: ErrorInfo) {
    // The message and where, never the data being drawn.
    console.error(`[${this.props.label}] render failed: ${error.message}`, info.componentStack ?? '');
  }

  componentDidUpdate(prev: Props) {
    if (this.state.failed && prev.resetKey !== this.props.resetKey) this.setState({ failed: false });
  }

  reset = () => this.setState({ failed: false });

  render() {
    return this.state.failed ? this.props.fallback(this.reset) : this.props.children;
  }
}

/** Dev only (`?crash=panes|header|app`, stripped from builds): throw during
 *  render to check the fallbacks. */
export function devCrash(where: string): boolean {
  return import.meta.env.DEV && new URLSearchParams(window.location.search).get('crash') === where;
}

export function DevCrash({ where }: { where: string }): null {
  if (devCrash(where)) throw new Error(`dev crash switch (${where})`);
  return null;
}
