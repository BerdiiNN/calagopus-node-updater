import { Component, type ErrorInfo, type ReactNode } from 'react';

/**
 * Silent error boundary (CLAUDE.md R4): an extension render error is logged, never shown,
 * and never allowed to take the host panel down with it.
 */
export default class SafeBoundary extends Component<{ children: ReactNode }, { failed: boolean }> {
  state = { failed: false };

  static getDerivedStateFromError() {
    return { failed: true };
  }

  componentDidCatch(error: Error, info: ErrorInfo) {
    console.error('[extension:com.pineriver.nodeupdater]', error, info.componentStack);
  }

  render() {
    return this.state.failed ? null : this.props.children;
  }
}

export { SafeBoundary };
