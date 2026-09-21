import { useEffect } from 'react';
import { useNavigate } from 'react-router';

/**
 * Toast actions and registry callbacks are plain functions, not components, so they cannot
 * call `useNavigate()`. This tiny component is mounted once through the global registry
 * (inside the router) and hands the navigate function to `navigateTo()`.
 */
let navigateRef: ((to: string) => void) | null = null;

export function NavigateBridge(): null {
  const navigate = useNavigate();

  useEffect(() => {
    navigateRef = (to) => navigate(to);
    return () => {
      navigateRef = null;
    };
  }, [navigate]);

  return null;
}

export function navigateTo(to: string): void {
  if (navigateRef) {
    navigateRef(to);
  } else {
    window.location.assign(to);
  }
}

export default NavigateBridge;
