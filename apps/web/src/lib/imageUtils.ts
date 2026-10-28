/**
 * 头像/图标图片压缩工具。
 * 上传前把任意图片压成 256×256 JPEG（中心裁剪），保证 < 200KB（服务器上限）。
 *
 * 返回 Blob（image/jpeg）而非 Uint8Array——TS 5.7 的 BlobPart 拒绝
 * Uint8Array<ArrayBufferLike>（SharedArrayBuffer 不兼容），直接返回 Blob 零转换。
 */

export async function compressToSquareJpeg(
  file: File | Blob,
  size: number = 256,
  quality: number = 0.85,
): Promise<Blob> {
  const bitmap = await createImageBitmap(file)
  try {
    const minSide = Math.min(bitmap.width, bitmap.height)
    const sx = (bitmap.width - minSide) / 2
    const sy = (bitmap.height - minSide) / 2
    const canvas = document.createElement('canvas')
    canvas.width = size
    canvas.height = size
    const ctx = canvas.getContext('2d')
    if (!ctx) throw new Error('canvas 2d context unavailable')
    ctx.drawImage(bitmap, sx, sy, minSide, minSide, 0, 0, size, size)
    return await new Promise<Blob>((resolve, reject) => {
      canvas.toBlob(
        (b) => (b ? resolve(b) : reject(new Error('toBlob returned null'))),
        'image/jpeg',
        quality,
      )
    })
  } finally {
    bitmap.close?.()
  }
}
