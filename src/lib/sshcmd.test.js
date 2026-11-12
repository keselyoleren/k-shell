import { describe, expect, it } from 'vitest'
import { matchHosts, parseSshCommand, toSshCommand } from './sshcmd'

const rule = (over) => ({
  kind: 'local',
  bind_host: '127.0.0.1',
  bind_port: 8080,
  dest_host: 'localhost',
  dest_port: 80,
  ...over,
})

describe('parseSshCommand', () => {
  it('reads one -L tunnel with the login and port', () => {
    const r = parseSshCommand('ssh -L 8080:localhost:80 deploy@example.com -p 2222')
    expect(r.errors).toEqual([])
    expect(r.user).toBe('deploy')
    expect(r.host).toBe('example.com')
    expect(r.port).toBe(2222)
    expect(r.forwards).toEqual([rule({})])
  })

  it('reads several tunnels from one command', () => {
    const r = parseSshCommand('ssh -L 8080:localhost:80 -R 9000:internal:22 -D 1080 jump')
    expect(r.errors).toEqual([])
    expect(r.forwards).toEqual([
      rule({}),
      rule({ kind: 'remote', bind_port: 9000, dest_host: 'internal', dest_port: 22 }),
      rule({ kind: 'dynamic', bind_port: 1080, dest_host: '', dest_port: 0 }),
    ])
  })

  it('accepts clustered flags and values attached to the flag', () => {
    const r = parseSshCommand('ssh -NL8080:localhost:80 host')
    expect(r.errors).toEqual([])
    expect(r.forwards).toEqual([rule({})])
  })

  it('keeps a bind address that is not loopback', () => {
    const r = parseSshCommand('ssh -L 0.0.0.0:8080:localhost:80 host')
    expect(r.forwards).toEqual([rule({ bind_host: '0.0.0.0' })])
  })

  it('reads a -D tunnel with only a port', () => {
    const r = parseSshCommand('ssh -D 1080 host')
    expect(r.forwards).toEqual([rule({ kind: 'dynamic', bind_port: 1080, dest_host: '', dest_port: 0 })])
  })

  it('keeps an IPv6 destination in brackets together', () => {
    const r = parseSshCommand('ssh -L 8080:[::1]:80 host')
    expect(r.errors).toEqual([])
    expect(r.forwards).toEqual([rule({ dest_host: '::1' })])
  })

  it('parses an ssh:// URL destination', () => {
    const r = parseSshCommand('ssh -L 8080:localhost:80 ssh://deploy@example.com:2222')
    expect(r.user).toBe('deploy')
    expect(r.host).toBe('example.com')
    expect(r.port).toBe(2222)
  })

  it('keeps a quoted destination with a space in one token', () => {
    const r = parseSshCommand(`ssh -L 8080:'my host':80 user@host`)
    expect(r.forwards).toEqual([rule({ dest_host: 'my host' })])
  })

  it('reports a missing destination and a missing tunnel', () => {
    const r = parseSshCommand('ssh -L 8080:localhost:80')
    expect(r.errors).toEqual(['No destination host (user@host) found'])

    const t = parseSshCommand('ssh host')
    expect(t.errors).toEqual(['No -L, -R or -D tunnel found'])
  })

  it('reports an invalid port instead of silently using the default', () => {
    const r = parseSshCommand('ssh -L 70000:localhost:80 host')
    expect(r.errors).toContain('-L 70000:localhost:80: invalid port or host')
    expect(r.forwards).toEqual([])
  })

  it('reports a flag that needs a value at the end of the line', () => {
    const r = parseSshCommand('ssh host -p')
    expect(r.errors).toContain('-p needs a value')
  })
})

describe('toSshCommand', () => {
  const host = { host: 'example.com', username: 'deploy', port: 22 }

  it('omits a loopback bind and the default port', () => {
    expect(toSshCommand(rule({}), host)).toBe('ssh -N -L 8080:localhost:80 deploy@example.com')
  })

  it('keeps a non-default bind address and port', () => {
    expect(toSshCommand(rule({ bind_host: '0.0.0.0', bind_port: 9000 }), { ...host, port: 2222 })).toBe(
      'ssh -N -L 0.0.0.0:9000:localhost:80 deploy@example.com -p 2222',
    )
  })

  it('writes a remote and a dynamic tunnel', () => {
    expect(toSshCommand(rule({ kind: 'remote', bind_port: 9000, dest_host: 'internal', dest_port: 22 }), host)).toBe(
      'ssh -N -R 9000:internal:22 deploy@example.com',
    )
    expect(toSshCommand(rule({ kind: 'dynamic', bind_port: 1080, dest_host: '', dest_port: 0 }), host)).toBe(
      'ssh -N -D 1080 deploy@example.com',
    )
  })

  it('returns an empty command without a host', () => {
    expect(toSshCommand(rule({}), null)).toBe('')
  })

  it('round-trips every tunnel kind back through the parser', () => {
    const cases = [
      rule({}),
      rule({ bind_host: '0.0.0.0', bind_port: 9000, dest_host: 'internal', dest_port: 22 }),
      rule({ kind: 'remote', bind_port: 9000, dest_host: 'internal', dest_port: 22 }),
      rule({ kind: 'dynamic', bind_port: 1080, dest_host: '', dest_port: 0 }),
      rule({ dest_host: '::1', dest_port: 22 }),
    ]
    for (const f of cases) {
      const back = parseSshCommand(toSshCommand(f, host))
      expect(back.errors).toEqual([])
      expect(back.user).toBe('deploy')
      expect(back.host).toBe('example.com')
      expect(back.forwards).toEqual([f])
    }
  })
})

describe('matchHosts', () => {
  const hosts = [
    { protocol: 'ssh', host: 'example.com', username: 'deploy', port: 2222 },
    { protocol: 'ssh', host: 'example.com', username: 'root', port: 22 },
    { protocol: 'sftp', host: 'example.com', username: 'deploy', port: 22 },
    { protocol: 'ssh', host: 'other.com', username: 'deploy', port: 22 },
  ]

  it('prefers an exact user, host and port match', () => {
    const parsed = { host: 'example.com', user: 'deploy', port: 2222 }
    expect(matchHosts(parsed, hosts)).toEqual([hosts[0]])
  })

  it('falls back to the same port when the user differs', () => {
    const parsed = { host: 'example.com', user: 'nobody', port: 22 }
    expect(matchHosts(parsed, hosts)).toEqual([hosts[1]])
  })

  it('falls back to every ssh host with that name', () => {
    const parsed = { host: 'example.com', user: 'deploy', port: 2200 }
    expect(matchHosts(parsed, hosts)).toEqual([hosts[0], hosts[1]])
  })
})
