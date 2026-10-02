import { useEffect, useState, type FormEvent } from 'react'
import { ConfigClient, type ConfigType, type ConfigValue } from './client'
import './ConfigEditor.css'

const client = new ConfigClient(import.meta.env.VITE_CONFIG_API ?? '')
const initialPath = '/platform_manager/io_devices[id = ECU0]/baud_rate'
const types: ConfigType[] = [
  'string', 'integer', 'float', 'boolean', 'object', 'array', 'str_list', 'numeric_list', 'null',
]

function inferValue(value: unknown): ConfigValue {
  if (value === null) return { type: 'null', value: null }
  if (typeof value === 'string') return { type: 'string', value }
  if (typeof value === 'boolean') return { type: 'boolean', value }
  if (typeof value === 'number') return { type: Number.isInteger(value) ? 'integer' : 'float', value }
  if (Array.isArray(value)) return { type: 'array', value: value.map(inferValue) }
  if (typeof value === 'object') {
    return {
      type: 'object',
      value: Object.fromEntries(Object.entries(value).map(([key, child]) => [key, inferValue(child)])),
    }
  }
  throw new Error('Unsupported value')
}

function formatValue(value: ConfigValue): string {
  if (value.type === 'object' || value.type === 'array') return JSON.stringify(toPlain(value), null, 2)
  if (value.type === 'str_list' || value.type === 'numeric_list') return (value.value as unknown[]).join('\n')
  return value.value === null ? 'null' : String(value.value)
}

function toPlain(value: ConfigValue): unknown {
  if (value.type === 'object') {
    return Object.fromEntries(
      Object.entries(value.value as Record<string, ConfigValue>).map(([key, child]) => [key, toPlain(child)]),
    )
  }
  if (value.type === 'array') return (value.value as ConfigValue[]).map(toPlain)
  return value.value
}

function parseValue(type: ConfigType, draft: string): ConfigValue {
  if (type === 'string') return { type, value: draft }
  if (type === 'integer') {
    if (!/^-?\d+$/.test(draft.trim())) throw new Error('Enter a whole number.')
    const value = Number(draft)
    if (!Number.isSafeInteger(value)) throw new Error('Integer is outside the safe range.')
    return { type, value }
  }
  if (type === 'float') {
    const value = Number(draft)
    if (!Number.isFinite(value)) throw new Error('Enter a finite number.')
    return { type, value }
  }
  if (type === 'boolean') {
    if (draft !== 'true' && draft !== 'false') throw new Error('Choose true or false.')
    return { type, value: draft === 'true' }
  }
  if (type === 'null') return { type, value: null }
  if (type === 'str_list') return { type, value: draft.split('\n').filter(Boolean) }
  if (type === 'numeric_list') {
    const values = draft.split('\n').filter((line) => line.trim()).map(Number)
    if (values.some((value) => !Number.isFinite(value))) throw new Error('Each line must be a finite number.')
    return { type, value: values }
  }
  const parsed: unknown = JSON.parse(draft)
  if (type === 'object' && (!parsed || Array.isArray(parsed) || typeof parsed !== 'object')) {
    throw new Error('Enter a JSON object.')
  }
  if (type === 'array' && !Array.isArray(parsed)) throw new Error('Enter a JSON array.')
  return inferValue(parsed)
}

function joinPath(parent: string, segment: string) {
  return parent === '/' ? `/${segment}` : `${parent.replace(/\/$/, '')}/${segment}`
}

type TreeNodeProps = {
  name: string
  path: string
  value: ConfigValue
  depth: number
  selectedPath: string
  onSelect(path: string): void
}

function TreeNode({ name, path, value, depth, selectedPath, onSelect }: TreeNodeProps) {
  const children: Array<[string, ConfigValue, string]> = []
  if (value.type === 'object') {
    for (const [key, child] of Object.entries(value.value as Record<string, ConfigValue>)) {
      children.push([key, child, joinPath(path, key)])
    }
  } else if (value.type === 'array') {
    ;(value.value as ConfigValue[]).forEach((child, index) => {
      const fields = child.type === 'object' ? child.value as Record<string, ConfigValue> : undefined
      const identifier = fields?.attr ?? fields?.id
      const selectedById = identifier?.type === 'string'
      const name = selectedById ? String(identifier.value) : String(index)
      const selector = selectedById ? `[id = ${name}]` : `[idx = ${index}]`
      children.push([name, child, `${path}${selector}`])
    })
  }

  if (children.length === 0) {
    return (
      <button
        className="tree-leaf"
        data-selected={selectedPath === path}
        style={{ paddingInlineStart: `${8 + depth * 14}px` }}
        type="button"
        onClick={() => onSelect(path)}
      >
        <span>{name}</span><small>{value.type}</small>
      </button>
    )
  }

  return (
    <details open={depth < 1}>
      <summary
        className="tree-branch"
        data-selected={selectedPath === path}
        style={{ paddingInlineStart: `${8 + depth * 14}px` }}
        onClick={() => onSelect(path)}
      >
        <span>{name}</span><small>{value.type} · {children.length}</small>
      </summary>
      <div>
        {children.map(([childName, childValue, childPath]) => (
          <TreeNode
            key={childPath}
            name={childName}
            path={childPath}
            value={childValue}
            depth={depth + 1}
            selectedPath={selectedPath}
            onSelect={onSelect}
          />
        ))}
      </div>
    </details>
  )
}

export default function ConfigEditor() {
  const [root, setRoot] = useState<ConfigValue | null>(null)
  const [path, setPath] = useState(initialPath)
  const [value, setValue] = useState<ConfigValue | null>(null)
  const [type, setType] = useState<ConfigType>('integer')
  const [draft, setDraft] = useState('')
  const [newPath, setNewPath] = useState('/platform_manager/test_setting')
  const [newType, setNewType] = useState<ConfigType>('string')
  const [newDraft, setNewDraft] = useState('')
  const [message, setMessage] = useState('')
  const [busy, setBusy] = useState(false)

  async function loadTree() {
    try {
      setRoot(await client.get('/'))
    } catch (error) {
      setMessage(error instanceof Error ? error.message : 'Unable to load settings.')
    }
  }

  async function loadValue(nextPath = path) {
    setPath(nextPath)
    setBusy(true)
    setMessage('')
    try {
      const nextValue = await client.get(nextPath)
      setValue(nextValue)
      setType(nextValue.type)
      setDraft(formatValue(nextValue))
    } catch (error) {
      setValue(null)
      setMessage(error instanceof Error ? error.message : 'Unable to read setting.')
    } finally {
      setBusy(false)
    }
  }

  async function saveValue() {
    setBusy(true)
    setMessage('')
    try {
      await client.set(path, parseValue(type, draft))
      await loadTree()
      await loadValue(path)
      setMessage('Saved to _delta.json.')
    } catch (error) {
      setMessage(error instanceof Error ? error.message : 'Unable to save setting.')
    } finally {
      setBusy(false)
    }
  }

  async function createValue(event: FormEvent<HTMLFormElement>) {
    event.preventDefault()
    setBusy(true)
    setMessage('')
    try {
      await client.set(newPath, parseValue(newType, newDraft))
      await loadTree()
      await loadValue(newPath)
      setMessage('Added to _delta.json.')
    } catch (error) {
      setMessage(error instanceof Error ? error.message : 'Unable to add setting.')
    } finally {
      setBusy(false)
    }
  }

  async function deleteValue() {
    setBusy(true)
    setMessage('')
    try {
      await client.delete(path)
      await loadTree()
      setValue(null)
      setMessage('Deleted; original marked unused in _delta.json.')
    } catch (error) {
      setMessage(error instanceof Error ? error.message : 'Unable to delete setting.')
    } finally {
      setBusy(false)
    }
  }

  async function restoreValue() {
    setBusy(true)
    setMessage('')
    try {
      await client.restore(path)
      await loadTree()
      await loadValue(path)
      setMessage('Restored original value; delta entry removed.')
    } catch (error) {
      setMessage(error instanceof Error ? error.message : 'Unable to restore setting.')
    } finally {
      setBusy(false)
    }
  }

  useEffect(() => {
    let active = true
    const refresh = async () => {
      try {
        const [nextRoot, nextValue] = await Promise.all([client.get('/'), client.get(path)])
        if (!active) return
        setRoot(nextRoot)
        setValue(nextValue)
        setType(nextValue.type)
        setDraft(formatValue(nextValue))
        setMessage('')
      } catch (error) {
        if (active) setMessage(error instanceof Error ? error.message : 'Unable to load settings.')
      }
    }
    const events = client.watch('/')
    events.onmessage = () => void refresh()
    void refresh()
    return () => {
      active = false
      events.close()
    }
  }, [path])

  return (
    <main className="config-app">
      <h1>rekv settings</h1>
      <div className="config-layout">
        <section className="tree-panel" aria-label="Settings tree">
          <h2>Settings</h2>
          <div className="tree-scroll">
            {root?.type === 'object' && Object.entries(root.value as Record<string, ConfigValue>).map(([name, child]) => (
              <TreeNode
                key={name}
                name={name}
                path={joinPath('/', name)}
                value={child}
                depth={0}
                selectedPath={path}
                onSelect={(selected) => void loadValue(selected)}
              />
            ))}
          </div>
        </section>

        <section className="setting-panel" aria-label="Selected setting">
          <form className="setting-form" onSubmit={(event) => { event.preventDefault(); void loadValue() }}>
            <label htmlFor="config-path">Path</label>
            <div className="path-row">
              <input id="config-path" value={path} onChange={(event) => setPath(event.target.value)} />
              <button type="submit" disabled={busy}>Get</button>
            </div>
          </form>

          {value && (
            <form className="setting-form" onSubmit={(event) => { event.preventDefault(); void saveValue() }}>
              <label htmlFor="config-type">Type</label>
              <select id="config-type" value={type} onChange={(event) => setType(event.target.value as ConfigType)}>
                {types.map((item) => <option key={item} value={item}>{item}</option>)}
              </select>
              <label htmlFor="config-value">Value</label>
              {type === 'boolean' ? (
                <select id="config-value" value={draft} onChange={(event) => setDraft(event.target.value)}>
                  <option value="true">true</option><option value="false">false</option>
                </select>
              ) : type === 'null' ? (
                <input id="config-value" value="null" readOnly />
              ) : (
                <textarea id="config-value" rows={type === 'object' || type === 'array' || type.endsWith('_list') ? 8 : 2} value={draft} onChange={(event) => setDraft(event.target.value)} />
              )}
              <div className="button-row">
                <button type="submit" disabled={busy}>Save</button>
                <button type="button" onClick={() => void deleteValue()} disabled={busy}>Delete</button>
                <button type="button" onClick={() => void restoreValue()} disabled={busy}>Restore original</button>
              </div>
            </form>
          )}

          <details className="add-panel">
            <summary>Add setting</summary>
            <form className="setting-form" onSubmit={(event) => void createValue(event)}>
              <label htmlFor="new-path">Path</label>
              <input id="new-path" value={newPath} onChange={(event) => setNewPath(event.target.value)} />
              <label htmlFor="new-type">Type</label>
              <select id="new-type" value={newType} onChange={(event) => setNewType(event.target.value as ConfigType)}>
                {types.map((item) => <option key={item} value={item}>{item}</option>)}
              </select>
              <label htmlFor="new-value">Initial value</label>
              <textarea id="new-value" rows={newType === 'object' || newType === 'array' || newType.endsWith('_list') ? 5 : 2} value={newDraft} onChange={(event) => setNewDraft(event.target.value)} />
              <button type="submit" disabled={busy}>Add</button>
            </form>
          </details>
          <p role="status">{message}</p>
        </section>
      </div>
    </main>
  )
}