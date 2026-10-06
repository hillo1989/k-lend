import { useMemo } from "react";

// Dekorativer BlockDAG-Hintergrund: viele kleine parallele Blöcke je Spalte,
// jeder verweist auf mehrere Blöcke der vorigen Spalte (links → rechts).
// Deterministisch erzeugt, damit das Bild bei jedem Laden gleich ist.

function prng(seed: number) {
  let s = seed >>> 0;
  return () => {
    s = (s * 1664525 + 1013904223) >>> 0;
    return s / 2 ** 32;
  };
}

interface Block {
  x: number;
  y: number;
  hot: boolean;
}

export function BlockDag({ columns = 22, height = 420 }: { columns?: number; height?: number }) {
  const { blocks, links, width } = useMemo(() => {
    const rnd = prng(20260928);
    const colGap = 64;
    const cols: Block[][] = [];
    for (let c = 0; c < columns; c++) {
      const n = 2 + Math.floor(rnd() * 3); // 2–4 parallele Blöcke
      const col: Block[] = [];
      const band = height - 80;
      for (let i = 0; i < n; i++) {
        const y = 40 + ((i + 0.5) / n) * band + (rnd() - 0.5) * 40;
        col.push({ x: 30 + c * colGap + (rnd() - 0.5) * 14, y, hot: rnd() < 0.12 });
      }
      cols.push(col);
    }
    const links: [Block, Block][] = [];
    for (let c = 1; c < cols.length; c++) {
      for (const b of cols[c]) {
        const prev = [...cols[c - 1]].sort((p, q) => Math.abs(p.y - b.y) - Math.abs(q.y - b.y));
        const k = 1 + Math.floor(rnd() * Math.min(3, prev.length));
        for (let i = 0; i < k; i++) links.push([prev[i], b]);
      }
    }
    return { blocks: cols.flat(), links, width: 60 + (columns - 1) * colGap };
  }, [columns, height]);

  return (
    <svg
      className="blockdag"
      viewBox={`0 0 ${width} ${height}`}
      preserveAspectRatio="xMidYMid slice"
      aria-hidden="true"
      focusable="false"
    >
      <g className="blockdag-links">
        {links.map(([a, b], i) => (
          <line key={i} x1={a.x + 9} y1={a.y} x2={b.x - 9} y2={b.y} />
        ))}
      </g>
      <g>
        {blocks.map((b, i) => (
          <rect
            key={i}
            className={b.hot ? "blockdag-block hot" : "blockdag-block"}
            x={b.x - 9}
            y={b.y - 9}
            width={18}
            height={18}
            rx={4}
            style={{ animationDelay: `${(i % 11) * 0.45}s` }}
          />
        ))}
      </g>
    </svg>
  );
}
