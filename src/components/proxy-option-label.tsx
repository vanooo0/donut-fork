import { FlagIcon } from "@/components/flag-icon";
import { Badge } from "@/components/ui/badge";
import { getProxyGeoLabel, getProxyHostPort } from "@/lib/proxy-utils";
import type { StoredProxy } from "@/types";

interface ProxyOptionLabelProps {
  proxy: StoredProxy;
}

// Two-line proxy entry used by every proxy picker: name + protocol badge
// on top, host:port and geo underneath, so identical names stay tellable
// apart and a proxy can be found by its address.
export function ProxyOptionLabel({ proxy }: ProxyOptionLabelProps) {
  const geo = getProxyGeoLabel(proxy);
  return (
    <div className="flex min-w-0 flex-1 flex-col">
      <div className="flex min-w-0 items-center gap-1.5">
        <span className="truncate">{proxy.name}</span>
        <Badge
          variant="outline"
          className="shrink-0 px-1 py-0 text-[10px] uppercase leading-tight"
        >
          {proxy.proxy_settings.proxy_type}
        </Badge>
      </div>
      <span className="flex min-w-0 items-center gap-1 font-mono text-xs text-muted-foreground">
        {proxy.geo_country ? (
          <FlagIcon countryCode={proxy.geo_country} className="shrink-0" />
        ) : null}
        <span className="truncate">
          {getProxyHostPort(proxy)}
          {geo ? ` · ${geo}` : ""}
        </span>
      </span>
    </div>
  );
}
