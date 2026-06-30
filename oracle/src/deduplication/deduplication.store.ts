import * as fs from 'fs';
import * as path from 'path';

export class DeduplicationStore {
  private seen: Set<string> = new Set();
  private filePath: string;

  constructor(storePath: string = path.join(__dirname, '../../data/seen-requests.json')) {
    this.filePath = storePath;
    this.loadFromDisk();
  }

  private loadFromDisk() {
    try {
      if (fs.existsSync(this.filePath)) {
        const data = JSON.parse(fs.readFileSync(this.filePath, 'utf8'));
        this.seen = new Set(data.seen || []);
      }
    } catch (error) {
      console.warn('Failed to load deduplication store, starting fresh:', error);
      // Ensure directory exists
      const dir = path.dirname(this.filePath);
      if (!fs.existsSync(dir)) {
        fs.mkdirSync(dir, { recursive: true });
      }
    }
  }

  private saveToDisk() {
    try {
      const dir = path.dirname(this.filePath);
      if (!fs.existsSync(dir)) {
        fs.mkdirSync(dir, { recursive: true });
      }
      fs.writeFileSync(this.filePath, JSON.stringify({ seen: Array.from(this.seen) }));
    } catch (error) {
      console.error('Failed to save deduplication store:', error);
    }
  }

  isDuplicate(requestId: bigint, raffleAddress: string): boolean {
    const key = `${raffleAddress}:${requestId.toString()}`;
    if (this.seen.has(key)) {
      return true;
    }
    this.seen.add(key);
    this.saveToDisk();
    return false;
  }
}
