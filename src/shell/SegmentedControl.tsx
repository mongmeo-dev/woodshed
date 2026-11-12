import { useId } from "react";
import { useGlide } from "./useGlide";

export interface SegmentedOption<T extends string> {
  value: T;
  label: string;
}

interface SegmentedControlProps<T extends string> {
  label: string;
  options: readonly SegmentedOption<T>[];
  value: T;
  onChange: (value: T) => void;
}

/** Single choice built on native radios, with a gliding selection pill. */
export function SegmentedControl<T extends string>({
  label,
  options,
  value,
  onChange,
}: SegmentedControlProps<T>) {
  const name = useId();
  const glide = useGlide<HTMLFieldSetElement>(`${value}:${options.map((o) => o.label).join("|")}`);

  return (
    <fieldset className="segmented" ref={glide.containerRef}>
      <legend className="visually-hidden">{label}</legend>
      <div
        aria-hidden="true"
        className="segmented-indicator"
        data-ready={glide.ready}
        style={glide.indicatorStyle}
      />
      {options.map((option) => {
        const checked = option.value === value;
        return (
          <label key={option.value} className="segmented-option" data-glide-active={checked}>
            <input
              type="radio"
              className="visually-hidden"
              name={name}
              value={option.value}
              checked={checked}
              onChange={() => onChange(option.value)}
            />
            {option.label}
          </label>
        );
      })}
    </fieldset>
  );
}
