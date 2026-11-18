/** T113-Pro connector and package illustration used in the basic board picker. */
export function BasicBoardDrawing() {
  return (
    <svg
      viewBox="0 0 255 164"
      className="basic-board-art"
      role="img"
      aria-label="T113-Pro 主板示意图"
    >
      <g fill="none" stroke="currentColor" strokeWidth="1.4" strokeLinejoin="round">
        <path fill="var(--color-selected)" d="M35 23h173l10 10v102l-8 8H36l-8-8V31z" />
        <g fill="var(--color-background)">
          <circle cx="40" cy="36" r="4" />
          <circle cx="204" cy="37" r="4" />
          <circle cx="40" cy="131" r="4" />
          <circle cx="204" cy="131" r="4" />
        </g>
        <rect x="68" y="30" width="117" height="16" rx="2" />
        {Array.from({ length: 16 }, (_, index) => 74 + 7 * index).map((x) => (
          <g key={x}>
            <circle cx={x} cy="35" r="1.5" />
            <circle cx={x} cy="41" r="1.5" />
          </g>
        ))}
        <g fill="var(--color-surface)">
          <rect x="22" y="54" width="29" height="17" rx="2" />
          <rect x="22" y="84" width="30" height="25" rx="2" />
          <path d="M25 88h22v17H25z" />
          <rect x="188" y="63" width="33" height="29" rx="2" />
          <path d="M192 67h26v20h-26z" />
          <rect x="174" y="108" width="47" height="13" rx="1" />
          <path d="M179 111h34v6h-34z" />
        </g>
        <rect x="106" y="79" width="33" height="33" rx="2" fill="var(--color-text)" />
        {Array.from({ length: 8 }, (_, i) => (
          <path key={i} d={`M${108 + 4 * i} 75v4m0 33v4M102 ${83 + 4 * i}h4m33 0h4`} />
        ))}
        <g stroke="var(--color-control-border)" strokeWidth="1">
          <path d="M51 62h24l13 13h15m-51 21h18l13-13h19m39 3h13l10-10h24m-48 20h13l17 17m-48-41V58l11-11" />
          <path d="M60 121h28l12-12m-44-5h21l11-12h12m36 17 12 13h25m-60-47-7-7H91" />
        </g>
        <g fill="var(--color-muted)">
          <rect x="65" y="76" width="14" height="9" />
          <rect x="87" y="52" width="23" height="11" />
          <rect x="150" y="53" width="18" height="9" />
          <rect x="149" y="118" width="10" height="6" />
          <rect x="61" y="110" width="12" height="6" />
          <rect x="156" y="88" width="13" height="9" />
        </g>
        <g fill="var(--color-sidebar)">
          <rect x="80" y="124" width="56" height="8" />
          <path d="M84 125v6m6-6v6m6-6v6m6-6v6m6-6v6m6-6v6m6-6v6m6-6v6" />
        </g>
      </g>
      <text
        x="122.5"
        y="98"
        textAnchor="middle"
        fill="var(--color-background)"
        fontFamily="var(--font-mono)"
        fontSize="7"
      >
        T113
      </text>
    </svg>
  );
}
