"use client";

import {
  createContext,
  type ReactNode,
  useContext,
  useEffect,
  useState,
} from "react";

type InputModality = "pointer" | "keyboard";

const InputModalityContext = createContext<InputModality>("keyboard");

export function InputModalityProvider({ children }: { children: ReactNode }) {
  const [modality, setModality] = useState<InputModality>("keyboard");

  useEffect(() => {
    const onPointerDown = () => setModality("pointer");
    const onKeyDown = (event: KeyboardEvent) => {
      if (!["Shift", "Control", "Alt", "Meta"].includes(event.key)) {
        setModality("keyboard");
      }
    };
    document.addEventListener("pointerdown", onPointerDown, true);
    document.addEventListener("keydown", onKeyDown, true);
    return () => {
      document.removeEventListener("pointerdown", onPointerDown, true);
      document.removeEventListener("keydown", onKeyDown, true);
    };
  }, []);

  return (
    <InputModalityContext.Provider value={modality}>
      {children}
    </InputModalityContext.Provider>
  );
}

export function useInputModality() {
  return useContext(InputModalityContext);
}
