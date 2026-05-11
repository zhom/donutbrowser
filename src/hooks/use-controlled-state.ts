import * as React from "react";

interface CommonControlledStateProps<T> {
  value?: T;
  defaultValue?: T;
}

/**
 * Returns either the caller-controlled `value` (read straight from props) or
 * an internal state when uncontrolled. The previous implementation kept the
 * controlled prop in a useEffect-synced state, which lagged one render
 * behind — when two sibling consumers flipped their `value` props in the
 * same React batch, both saw stale state for one render and the wrong tree
 * mounted briefly. Returning the prop directly when controlled makes the
 * component synchronous in the controlled case, matching React's controlled
 * input pattern.
 */
export function useControlledState<T, Rest extends unknown[] = []>(
  props: CommonControlledStateProps<T> & {
    onChange?: (value: T, ...args: Rest) => void;
  },
): readonly [T, (next: T, ...args: Rest) => void] {
  const { value, defaultValue, onChange } = props;

  const [internalState, setInternalState] = React.useState<T>(
    value ?? (defaultValue as T),
  );

  const isControlled = value !== undefined;
  const currentState = isControlled ? value : internalState;

  const setState = React.useCallback(
    (next: T, ...args: Rest) => {
      // Always notify caller via onChange so a controlled consumer can
      // update its own state. Internal state is only relevant in the
      // uncontrolled case but we keep it in sync so the hook reads the
      // right value if the consumer later removes its controlled prop.
      if (!isControlled) {
        setInternalState(next);
      }
      onChange?.(next, ...args);
    },
    [isControlled, onChange],
  );

  return [currentState, setState] as const;
}
