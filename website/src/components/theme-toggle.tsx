'use client';

import { useEffect, useState } from 'react';

export function ThemeToggle() {
  const [dark, setDark] = useState<boolean | null>(null);

  useEffect(() => {
    const media = matchMedia('(prefers-color-scheme: dark)');
    function syncTheme() {
      const saved = document.documentElement.dataset.theme;
      setDark(saved ? saved === 'dark' : media.matches);
    }
    syncTheme();
    media.addEventListener('change', syncTheme);
    return () => media.removeEventListener('change', syncTheme);
  }, []);

  function toggle() {
    const theme = dark ? 'light' : 'dark';
    document.documentElement.dataset.theme = theme;
    setDark(theme === 'dark');
    try {
      localStorage.setItem('crabhosts.theme', theme);
    } catch {
      // The selection still applies for this visit.
    }
  }

  const label = dark ? '切换到浅色主题' : '切换到暗黑主题';
  return (
    <button id="theme" className="theme" type="button" hidden={dark === null}
      aria-label={label} title={label} onClick={toggle}>
      {dark ? '☀' : '☾'}
    </button>
  );
}
