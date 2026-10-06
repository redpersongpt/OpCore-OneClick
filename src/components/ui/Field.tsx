import { useId, useState, type ReactNode } from 'react';

const CONTROL =
  'h-8 w-full rounded-md border border-line bg-panel-2 px-2.5 text-base text-fg placeholder:text-fg-4 ' +
  'hover:border-line-strong focus:border-accent focus:outline-none disabled:cursor-not-allowed disabled:opacity-50';

export function Field({
  label,
  hint,
  error,
  children,
  className = '',
}: {
  label: ReactNode;
  hint?: ReactNode;
  error?: ReactNode;
  children: (id: string) => ReactNode;
  className?: string;
}) {
  const id = useId();
  return (
    <div className={`flex min-w-0 flex-col gap-1 ${className}`}>
      <label htmlFor={id} className="text-sm font-medium text-fg-2">
        {label}
      </label>
      {children(id)}
      {error ? (
        <p className="text-xs text-err-fg">{error}</p>
      ) : hint ? (
        <p className="text-xs text-fg-3">{hint}</p>
      ) : null}
    </div>
  );
}

export function TextInput({
  id,
  value,
  onChange,
  placeholder,
  disabled,
  mono = false,
  ariaLabel,
}: {
  id?: string;
  value: string;
  onChange: (value: string) => void;
  placeholder?: string;
  disabled?: boolean;
  mono?: boolean;
  ariaLabel?: string;
}) {
  return (
    <input
      id={id}
      type="text"
      value={value}
      placeholder={placeholder}
      disabled={disabled}
      aria-label={ariaLabel}
      spellCheck={false}
      autoComplete="off"
      onChange={(e) => onChange(e.target.value)}
      className={`${CONTROL} ${mono ? 'font-mono text-sm' : ''}`}
    />
  );
}

/**
 * Text field bound to a parsed value. Invalid text is kept on screen (marked
 * invalid) without being committed; empty text commits null.
 */
export function ParsedInput<T>({
  id,
  value,
  format,
  parse,
  onChange,
  placeholder,
  disabled,
  mono = false,
  ariaLabel,
}: {
  id?: string;
  value: T | null;
  format: (value: T) => string;
  parse: (text: string) => T | null;
  onChange: (value: T | null) => void;
  placeholder?: string;
  disabled?: boolean;
  mono?: boolean;
  ariaLabel?: string;
}) {
  const shown = value === null ? '' : format(value);
  const [text, setText] = useState(shown);
  const [synced, setSynced] = useState(shown);
  if (shown !== synced) {
    setSynced(shown);
    setText(shown);
  }
  const invalid = text.trim() !== '' && parse(text) === null;

  return (
    <input
      id={id}
      type="text"
      inputMode={mono ? 'text' : 'numeric'}
      value={text}
      placeholder={placeholder}
      disabled={disabled}
      aria-label={ariaLabel}
      aria-invalid={invalid || undefined}
      spellCheck={false}
      autoComplete="off"
      onChange={(e) => {
        const next = e.target.value;
        setText(next);
        if (next.trim() === '') onChange(null);
        else {
          const parsed = parse(next);
          if (parsed !== null) onChange(parsed);
        }
      }}
      className={`${CONTROL} ${mono ? 'font-mono text-sm' : ''} ${invalid ? 'border-err-line focus:border-err' : ''}`}
    />
  );
}

export interface SelectOption<V extends string> {
  value: V;
  label: string;
}

export function Select<V extends string>({
  id,
  value,
  options,
  onChange,
  disabled,
  ariaLabel,
}: {
  id?: string;
  value: V;
  options: readonly SelectOption<V>[];
  onChange: (value: V) => void;
  disabled?: boolean;
  ariaLabel?: string;
}) {
  const known = options.some((o) => o.value === value);
  return (
    <select
      id={id}
      value={value}
      disabled={disabled}
      aria-label={ariaLabel}
      onChange={(e) => onChange(e.target.value as V)}
      className={`${CONTROL} cursor-pointer pr-7`}
    >
      {!known && <option value={value}>{value}</option>}
      {options.map((o) => (
        <option key={o.value} value={o.value}>
          {o.label}
        </option>
      ))}
    </select>
  );
}

export function Toggle({
  checked,
  onChange,
  label,
  description,
  disabled,
}: {
  checked: boolean;
  onChange: (checked: boolean) => void;
  label: ReactNode;
  description?: ReactNode;
  disabled?: boolean;
}) {
  const id = useId();
  return (
    <div className="flex items-start gap-3 py-1">
      <button
        id={id}
        type="button"
        role="switch"
        aria-checked={checked}
        disabled={disabled}
        onClick={() => onChange(!checked)}
        className={`relative mt-0.5 inline-flex h-4.5 w-8 shrink-0 items-center rounded-full border transition-colors disabled:cursor-not-allowed disabled:opacity-40 ${
          checked ? 'border-accent bg-accent' : 'border-line-strong bg-panel-3'
        }`}
      >
        <span
          className={`inline-block size-3.5 rounded-full bg-white shadow transition-transform ${
            checked ? 'translate-x-3.5' : 'translate-x-0.5'
          }`}
          aria-hidden
        />
      </button>
      <label htmlFor={id} className="min-w-0 cursor-pointer">
        <span className="block text-base text-fg">{label}</span>
        {description && <span className="block text-xs text-fg-3">{description}</span>}
      </label>
    </div>
  );
}

export function Checkbox({
  checked,
  onChange,
  label,
  description,
  disabled,
}: {
  checked: boolean;
  onChange: (checked: boolean) => void;
  label: ReactNode;
  description?: ReactNode;
  disabled?: boolean;
}) {
  const id = useId();
  return (
    <div className="flex items-start gap-2.5 py-1">
      <input
        id={id}
        type="checkbox"
        checked={checked}
        disabled={disabled}
        onChange={(e) => onChange(e.target.checked)}
        className="mt-0.5 size-3.5 shrink-0 cursor-pointer accent-accent"
      />
      <label htmlFor={id} className="min-w-0 cursor-pointer">
        <span className="block text-base text-fg">{label}</span>
        {description && <span className="block text-xs text-fg-3">{description}</span>}
      </label>
    </div>
  );
}
