/** One page of the site. */
export interface PageSpec {
  title: string;
  body: string;
}

/**
 * Return the page for a hash route path ("/", "/pricing", ...), or null for
 * unknown routes. TODO: implement per the requirements given in this session.
 */
export function routeFor(path: string): PageSpec | null {
  void path;
  return null;
}
