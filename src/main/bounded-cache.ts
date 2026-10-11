/** Retain reusable data across games without retaining every previously visited version. */
export class BoundedCache<K, V> extends Map<K, V> {
  private readonly capacity: number
  constructor(capacity: number) {
    super()
    if (!Number.isSafeInteger(capacity) || capacity < 1) throw new Error('Invalid cache capacity')
    this.capacity = capacity
  }

  override get(key: K): V | undefined {
    const value = super.get(key)
    if (super.has(key)) { super.delete(key); super.set(key, value!) }
    return value
  }

  override set(key: K, value: V): this {
    super.delete(key)
    super.set(key, value)
    if (this.size > this.capacity) super.delete(this.keys().next().value!)
    return this
  }
}
