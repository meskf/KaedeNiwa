const fs = require('fs')
const log = []

// en.ts
let en = fs.readFileSync('src/locales/en.ts', 'utf8')
const enAnchor = `  search: {`
const enAdd = `  channel: {
    createTitle: 'Create Channel',
    createName: 'Channel name',
    createKind: 'Channel type',
    kindText: 'Text',
    kindVoice: 'Voice',
    createCategory: 'Category',
    uncategorized: 'Uncategorized',
    isPrivate: 'Private channel (password)',
    password: 'Password',
    createSubmit: 'Create',
    createCancel: 'Cancel',
    createPlaceholder: 'new-channel',
    settingsTitle: 'Channel Settings',
    settingsName: 'Name',
    settingsCategory: 'Category',
    settingsSave: 'Save',
    settingsCancel: 'Cancel',
    settingsDelete: 'Delete channel',
    deleteConfirm: 'Delete this channel? Messages will be removed. This cannot be undone.',
    createFailed: 'Failed to create channel',
    updateFailed: 'Failed to update channel',
    deleteFailed: 'Failed to delete channel',
    addChannel: 'Add Channel',
    openSettings: 'Channel settings',
  },
  search: {`
if (!en.includes('channel: {')) {
  en = en.replace(enAnchor, enAdd)
  fs.writeFileSync('src/locales/en.ts', en)
  log.push('OK en.ts')
} else log.push('SKIP en.ts')

// zh.ts
let zh = fs.readFileSync('src/locales/zh.ts', 'utf8')
const zhAdd = `  channel: {
    createTitle: '创建频道',
    createName: '频道名',
    createKind: '频道类型',
    kindText: '文本',
    kindVoice: '语音',
    createCategory: '分类',
    uncategorized: '未分类',
    isPrivate: '私密频道（密码）',
    password: '密码',
    createSubmit: '创建',
    createCancel: '取消',
    createPlaceholder: '新频道',
    settingsTitle: '频道设置',
    settingsName: '名称',
    settingsCategory: '分类',
    settingsSave: '保存',
    settingsCancel: '取消',
    settingsDelete: '删除频道',
    deleteConfirm: '确定删除该频道？消息将一并删除，无法恢复。',
    createFailed: '创建频道失败',
    updateFailed: '更新频道失败',
    deleteFailed: '删除频道失败',
    addChannel: '创建频道',
    openSettings: '频道设置',
  },
  search: {`
if (!zh.includes('channel: {')) {
  zh = zh.replace(zhAnchor, zhAdd)
  fs.writeFileSync('src/locales/zh.ts', zh)
  log.push('OK zh.ts')
} else log.push('SKIP zh.ts')

log.forEach(l => console.log(l))
