const fs = require('fs')
let s = fs.readFileSync('src/components/SearchPalette.tsx', 'utf8')
s = s.replace(
  "import { useEffect, useMemo, useRef, useState } from 'react'",
  "import { useEffect, useMemo, useRef, useState, type ReactNode } from 'react'"
)
s = s.replace('function highlight(text: string, q: string): JSX.Element {', 'function highlight(text: string, q: string): ReactNode {')
s = s.replace('  const parts: JSX.Element[] = []', '  const parts: ReactNode[] = []')
fs.writeFileSync('src/components/SearchPalette.tsx', s)
console.log('OK: JSX -> ReactNode')
