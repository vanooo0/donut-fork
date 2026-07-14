import type { StoredProxy } from "@/types";

export function getProxyHostPort(proxy: StoredProxy): string {
  return `${proxy.proxy_settings.host}:${proxy.proxy_settings.port}`;
}

export function getProxyGeoLabel(proxy: StoredProxy): string {
  return [proxy.geo_country, proxy.geo_city].filter(Boolean).join(", ");
}

export interface ParsedProxyString {
  proxy_type?: string;
  host: string;
  port: number;
  username?: string;
  password?: string;
}

const KNOWN_PROXY_TYPES = new Set(["http", "https", "socks4", "socks5", "ss"]);

function normalizeProxyType(scheme: string): string | undefined {
  const lower = scheme.toLowerCase();
  if (lower === "socks") return "socks5";
  return KNOWN_PROXY_TYPES.has(lower) ? lower : undefined;
}

function isValidPort(value: string): boolean {
  if (!/^\d+$/.test(value)) return false;
  const port = Number.parseInt(value, 10);
  return port > 0 && port <= 65535;
}

// Parses a single pasted proxy string. Accepted shapes:
//   [scheme://][user:pass@]host:port
//   host:port:user:pass
//   user:pass:host:port
export function parseProxyString(raw: string): ParsedProxyString | null {
  const text = raw.trim();
  if (!text || /\s/.test(text)) return null;

  let proxyType: string | undefined;
  let rest = text;
  const schemeMatch = /^([a-z0-9]+):\/\//i.exec(rest);
  if (schemeMatch) {
    proxyType = normalizeProxyType(schemeMatch[1]);
    rest = rest.slice(schemeMatch[0].length);
  }

  let username: string | undefined;
  let password: string | undefined;
  let hostPart = rest;
  const at = rest.lastIndexOf("@");
  if (at !== -1) {
    const creds = rest.slice(0, at);
    hostPart = rest.slice(at + 1);
    const sep = creds.indexOf(":");
    if (sep === -1) {
      username = creds;
    } else {
      username = creds.slice(0, sep);
      password = creds.slice(sep + 1);
    }
  }

  const parts = hostPart.split(":");
  let host: string;
  let port: string;
  if (parts.length === 2) {
    [host, port] = parts;
  } else if (username === undefined && parts.length === 4) {
    if (isValidPort(parts[1])) {
      [host, port, username, password] = parts;
    } else if (isValidPort(parts[3])) {
      [username, password, host, port] = parts;
    } else {
      return null;
    }
  } else {
    return null;
  }

  if (!host || !isValidPort(port)) return null;

  return {
    proxy_type: proxyType,
    host,
    port: Number.parseInt(port, 10),
    username: username || undefined,
    password: password || undefined,
  };
}

// Value handed to cmdk so typing a host, port, protocol, username, or
// geo fragment matches the proxy — not just its display name.
export function getProxySearchValue(proxy: StoredProxy): string {
  return [
    proxy.name,
    getProxyHostPort(proxy),
    proxy.proxy_settings.proxy_type,
    proxy.proxy_settings.username,
    proxy.geo_country,
    proxy.geo_city,
    proxy.geo_isp,
  ]
    .filter(Boolean)
    .join(" ");
}
