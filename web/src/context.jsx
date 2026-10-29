import { createContext, useContext, useRef, useState } from "react";
export const AppContext = createContext(null);
export function useApp() {
  return useContext(AppContext);
}
export function useDraft(key, initial = "") {
  const { drafts } = useApp();
  const [value, setValue] = useState(() => drafts.current.get(key) ?? initial);
  const keyRef = useRef(key);
  if (keyRef.current !== key) {
    keyRef.current = key;
    setValue(drafts.current.get(key) ?? initial);
  }
  const set = (next) =>
    setValue((old) => {
      const value = typeof next === "function" ? next(old) : next;
      drafts.current.set(key, value);
      return value;
    });
  return [value, set];
}
