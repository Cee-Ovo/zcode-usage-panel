/** Small, consistent outline icons for the desktop navigation. */
export function NavIcon({ name }: { name: "dashboard" | "sessions" | "models" | "settings" }) {
  return (
    <svg width="18" height="18" viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="1.6" strokeLinecap="round" strokeLinejoin="round" aria-hidden="true">
      {name === "dashboard" && <><rect x="3" y="3" width="7" height="7" rx="2" /><rect x="14" y="3" width="7" height="11" rx="2" /><rect x="3" y="14" width="7" height="7" rx="2" /><rect x="14" y="18" width="7" height="3" rx="1.5" /></>}
      {name === "sessions" && <><rect x="5" y="6" width="15" height="15" rx="3" /><path d="M16 3H6a3 3 0 0 0-3 3v10M9 11h7M9 15h5" /></>}
      {name === "models" && <><path d="m12 3 9 5-9 5-9-5 9-5ZM3 12l9 5 9-5M3 16l9 5 9-5" /></>}
      {name === "settings" && <><path d="M4 6h16M4 12h16M4 18h16" /><circle cx="9" cy="6" r="2" fill="var(--refine-surface, white)" /><circle cx="15" cy="12" r="2" fill="var(--refine-surface, white)" /><circle cx="9" cy="18" r="2" fill="var(--refine-surface, white)" /></>}
    </svg>
  );
}
