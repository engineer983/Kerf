/**
 * The browser version's stand-in for `@tauri-apps/api/event`. Events come
 * from a connected machine, and a page has none, so nothing is ever heard.
 */
export async function listen<T>(_event: string, _handler: (event: { payload: T }) => void): Promise<() => void> {
  return () => {};
}