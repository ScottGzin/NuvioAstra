// Sidebar icons: simple line icons (one stroke width, rounded joins, `currentColor`), so the
// sidebar looks like a desktop app rather than a set of glossy illustrations. The color comes from
// CSS: muted when idle, the app's cyan when selected.

const Svg = ({ children }: { children: React.ReactNode }) => (
  <svg viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="1.8" strokeLinecap="round" strokeLinejoin="round" aria-hidden>
    {children}
  </svg>
);

export const HomeLine = () => (
  <Svg>
    <path d="M3.5 10.5 12 3.5l8.5 7" />
    <path d="M5.5 9v10.5a1 1 0 0 0 1 1H10v-6h4v6h3.5a1 1 0 0 0 1-1V9" />
  </Svg>
);

export const TvLine = () => (
  <Svg>
    <rect x="3" y="6.5" width="18" height="12.5" rx="2.5" />
    <path d="m8.5 2.5 3.5 4 3.5-4" />
    <path d="M10.5 10.5v5l4-2.5z" />
  </Svg>
);

export const SearchLine = () => (
  <Svg>
    <circle cx="11" cy="11" r="6.5" />
    <path d="m16 16 4.5 4.5" />
  </Svg>
);

export const LibraryLine = () => (
  <Svg>
    <rect x="3.5" y="3.5" width="12" height="12" rx="2" />
    <path d="M8.5 20.5H18a2.5 2.5 0 0 0 2.5-2.5V8.5" />
  </Svg>
);

export const SettingsLine = () => (
  <Svg>
    <circle cx="12" cy="12" r="3" />
    <path d="M19.4 15a1.7 1.7 0 0 0 .3 1.8l.1.1a2 2 0 1 1-2.8 2.8l-.1-.1a1.7 1.7 0 0 0-1.8-.3 1.7 1.7 0 0 0-1 1.5V21a2 2 0 1 1-4 0v-.1a1.7 1.7 0 0 0-1.1-1.5 1.7 1.7 0 0 0-1.8.3l-.1.1a2 2 0 1 1-2.8-2.8l.1-.1a1.7 1.7 0 0 0 .3-1.8 1.7 1.7 0 0 0-1.5-1H3a2 2 0 1 1 0-4h.1a1.7 1.7 0 0 0 1.5-1.1 1.7 1.7 0 0 0-.3-1.8l-.1-.1a2 2 0 1 1 2.8-2.8l.1.1a1.7 1.7 0 0 0 1.8.3H9a1.7 1.7 0 0 0 1-1.5V3a2 2 0 1 1 4 0v.1a1.7 1.7 0 0 0 1 1.5 1.7 1.7 0 0 0 1.8-.3l.1-.1a2 2 0 1 1 2.8 2.8l-.1.1a1.7 1.7 0 0 0-.3 1.8V9a1.7 1.7 0 0 0 1.5 1H21a2 2 0 1 1 0 4h-.1a1.7 1.7 0 0 0-1.5 1z" />
  </Svg>
);

// ---------- Settings sections (same line style) ----------

export const AccountLine = () => (
  <Svg>
    <path d="M7 18.5a4.5 4.5 0 0 1-.6-9 6 6 0 0 1 11.4 1.6A3.8 3.8 0 0 1 17.5 18.5z" />
    <path d="M12 15.5v-5m-2.2 2.2L12 10.5l2.2 2.2" />
  </Svg>
);

export const AddonsLine = () => (
  <Svg>
    <path d="M4.5 8h3.2a2.3 2.3 0 1 1 4.6 0h3.2v3.2a2.3 2.3 0 1 1 0 4.6v3.2h-3.2a2.3 2.3 0 1 0-4.6 0H4.5v-3.2a2.3 2.3 0 1 0 0-4.6z" />
  </Svg>
);

export const PlaybackLine = () => (
  <Svg>
    <circle cx="12" cy="12" r="8.5" />
    <path d="M10.3 8.8v6.4l5-3.2z" />
  </Svg>
);

export const P2pLine = () => (
  <Svg>
    <path d="M8 20V5m-3.5 3.5L8 5l3.5 3.5" />
    <path d="M16 4v15m-3.5-3.5L16 19l3.5-3.5" />
  </Svg>
);

export const AudioLine = () => (
  <Svg>
    <path d="M4 9.5h3.5L12 5.5v13l-4.5-4H4z" />
    <path d="M15.5 9a4.2 4.2 0 0 1 0 6" />
    <path d="M18.3 6.3a8 8 0 0 1 0 11.4" />
  </Svg>
);

export const SubtitlesLine = () => (
  <Svg>
    <rect x="3" y="5.5" width="18" height="13" rx="2.5" />
    <path d="M7 12.5h3.5M13 12.5h4M7 15.5h7" />
  </Svg>
);

export const ProfileDataLine = () => (
  <Svg>
    <path d="M3.5 7.5a2 2 0 0 1 2-2h4l2 2h7a2 2 0 0 1 2 2v8.5a2 2 0 0 1-2 2h-13a2 2 0 0 1-2-2z" />
    <circle cx="12" cy="12.3" r="1.9" />
    <path d="M8.8 17.3a3.4 3.4 0 0 1 6.4 0" />
  </Svg>
);

export const UpdatesLine = () => (
  <Svg>
    <path d="M19.5 11A7.5 7.5 0 0 0 6 6.6L4.5 8" />
    <path d="M4.5 3.5V8H9" />
    <path d="M4.5 13A7.5 7.5 0 0 0 18 17.4l1.5-1.4" />
    <path d="M19.5 20.5V16H15" />
  </Svg>
);

export const ServicesLine = () => (
  <Svg>
    <path d="M10 13.5a4.5 4.5 0 0 0 6.4 0l2.3-2.3a4.5 4.5 0 0 0-6.4-6.4l-1.2 1.2" />
    <path d="M14 10.5a4.5 4.5 0 0 0-6.4 0l-2.3 2.3a4.5 4.5 0 0 0 6.4 6.4l1.2-1.2" />
  </Svg>
);

export const PendingLine = () => (
  <Svg>
    <circle cx="12" cy="12" r="8.5" />
    <path d="M12 7.5V12l3 2" />
  </Svg>
);
