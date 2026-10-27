const fs = require('fs')
let s = fs.readFileSync('src/stores/chat.ts', 'utf8')
const old = `export interface ChannelInfo {
  id: string;
  name: string;
  kind: string;
  position: number;
  is_private: boolean;
}`
const new_ = `export interface ChannelInfo {
  id: string;
  name: string;
  kind: string;
  position: number;
  is_private: boolean;
  category_id?: string | null;
}`
if (!s.includes(old)) { console.error('FAIL'); process.exit(1) }
s = s.replace(old, new_)
fs.writeFileSync('src/stores/chat.ts', s)
console.log('OK ChannelInfo.category_id')
