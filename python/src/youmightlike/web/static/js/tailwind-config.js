// Tailwind CDN configuration
tailwind.config = {
  theme: {
    extend: {
      fontFamily: {
        sans: ['-apple-system', 'BlinkMacSystemFont', '"SF Pro Text"', '"SF Pro Display"', '"SF Pro"', 'system-ui', '"Segoe UI"', 'Roboto', 'Helvetica', 'Arial', 'sans-serif'],
        mono: ['"JetBrains Mono"', 'ui-monospace', 'monospace'],
      },
      colors: {
        apple: {
          bg: '#fbfbfd',
          card: '#ffffff',
          sidebar: '#f5f5f7',
          border: '#e5e5ea',
          subtle: '#f2f2f7',
          text: '#1d1d1f',
          muted: '#86868b',
          blue: '#0071e3',
          blueHover: '#0077ed',
        },
        canvas: {
          bg: '#fbfbfd',
          panel: '#ffffff',
          sidebar: '#f5f5f7',
          border: '#e5e5ea',
          borderDark: '#d1d1d6',
          text: '#1d1d1f',
          muted: '#86868b',
        }
      }
    }
  }
};
