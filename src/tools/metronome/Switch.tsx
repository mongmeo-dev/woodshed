interface SwitchProps {
  checked: boolean;
  onChange: (checked: boolean) => void;
  labelledBy: string;
}

/** On/off switch (component.gallery "Toggle": a button with role="switch"). */
export function Switch({ checked, onChange, labelledBy }: SwitchProps) {
  return (
    <button
      type="button"
      role="switch"
      aria-checked={checked}
      aria-labelledby={labelledBy}
      className="switch"
      onClick={() => onChange(!checked)}
    >
      <span className="switch-knob" />
    </button>
  );
}
