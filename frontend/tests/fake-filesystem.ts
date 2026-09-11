export class MemoryFile {
  data = new Uint8Array()
  constructor(public name: string) {}
  async createWritable({ keepExistingData = false } = {}) {
    let data = keepExistingData ? this.data.slice() : new Uint8Array()
    let position = 0
    return {
      write: async (value: Uint8Array | Blob | string | { type: 'seek'; position: number }) => {
        if (typeof value === 'object' && 'type' in value && value.type === 'seek') { position = value.position; return }
        const bytes = value instanceof Uint8Array ? value : value instanceof Blob ? new Uint8Array(await value.arrayBuffer()) : new TextEncoder().encode(String(value))
        const next = new Uint8Array(Math.max(data.length, position + bytes.length))
        next.set(data); next.set(bytes, position); data = next; position += bytes.length
      },
      close: async () => { this.data = data },
      abort: async () => {},
    }
  }
}

export class MemoryDirectory {
  entries = new Map<string, MemoryDirectory | MemoryFile>()
  constructor(public name: string) {}
  async getDirectoryHandle(name: string, { create = false } = {}) {
    let entry = this.entries.get(name)
    if (entry instanceof MemoryFile) throw new DOMException('File at directory path', 'TypeMismatchError')
    if (!entry && create) { entry = new MemoryDirectory(name); this.entries.set(name, entry) }
    if (!entry) throw new DOMException('Not found', 'NotFoundError')
    return entry as MemoryDirectory
  }
  async getFileHandle(name: string, { create = false } = {}) {
    let entry = this.entries.get(name)
    if (entry instanceof MemoryDirectory) throw new DOMException('Directory at file path', 'TypeMismatchError')
    if (!entry && create) { entry = new MemoryFile(name); this.entries.set(name, entry) }
    if (!entry) throw new DOMException('Not found', 'NotFoundError')
    return entry as MemoryFile
  }
  files(prefix = ''): Array<{ path: string; file: MemoryFile }> {
    return [...this.entries].flatMap(([name, entry]) => entry instanceof MemoryDirectory
      ? entry.files(`${prefix}${name}/`) : [{ path: `${prefix}${name}`, file: entry }])
  }
  handle() { return this as unknown as FileSystemDirectoryHandle }
}
