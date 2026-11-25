export function isWorkshopLink(value: string): boolean {
  try {
    const url = new URL(value);
    if (!["http:", "https:"].includes(url.protocol)
      || !["steamcommunity.com", "www.steamcommunity.com"].includes(url.hostname)
      || url.port || url.username || url.password
      || !/^\/(sharedfiles|workshop)\/filedetails\/?$/.test(url.pathname)) return false;

    const ids = url.searchParams.getAll("id");
    if (ids.length !== 1 || !/^[0-9]+$/.test(ids[0])) return false;
    const id = BigInt(ids[0]);
    return id > 0n && id <= 18_446_744_073_709_551_615n;
  } catch {
    return false;
  }
}
