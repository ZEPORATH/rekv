export type ConfigType =
  | 'string'
  | 'integer'
  | 'float'
  | 'boolean'
  | 'object'
  | 'array'
  | 'str_list'
  | 'numeric_list'
  | 'null'

export type ConfigValue = { type: ConfigType; value: unknown }
type RpcResponse<T> = {
  id: number
  ok: boolean
  result?: T
  error?: { code: string; message: string }
}

export class ConfigClient {
  private readonly baseUrl: string

  constructor(baseUrl = '') {
    this.baseUrl = baseUrl
  }

  async get(path: string): Promise<ConfigValue> {
    return this.request<ConfigValue>(`/api/config?${new URLSearchParams({ path })}`)
  }

  async set(path: string, value: ConfigValue): Promise<void> {
    await this.request<void>('/api/config', {
      method: 'POST',
      headers: { 'Content-Type': 'application/json' },
      body: JSON.stringify({ id: Date.now(), path, value }),
    })
  }

  async delete(path: string): Promise<void> {
    await this.request<void>(`/api/config?${new URLSearchParams({ path })}`, { method: 'DELETE' })
  }

  async restore(path: string): Promise<void> {
    await this.request<void>('/api/config/restore', {
      method: 'POST',
      headers: { 'Content-Type': 'application/json' },
      body: JSON.stringify({ id: Date.now(), path }),
    })
  }

  async list(path: string): Promise<string[]> {
    return this.request<string[]>(`/api/config/list?${new URLSearchParams({ path })}`)
  }

  watch(path: string): EventSource {
    const query = new URLSearchParams({ path })
    return new EventSource(`${this.baseUrl}/api/events?${query}`)
  }

  private async request<T>(path: string, init?: RequestInit): Promise<T> {
    const response = await fetch(`${this.baseUrl}${path}`, init)
    const body = (await response.json()) as RpcResponse<T>
    if (!response.ok || !body.ok) {
      throw new Error(body.error?.message ?? `Request failed (${response.status})`)
    }
    return body.result as T
  }
}