import type { ReactNode } from 'react';

import { Alert, Button, Checkbox, Label, Spinner, Switch, cn } from '@heroui/react';
import { CircleAlert } from 'lucide-react';

export function PageHeader({
  title,
  subtitle,
  leading,
  actions,
  className,
}: {
  title: ReactNode;
  subtitle?: ReactNode;
  leading?: ReactNode;
  actions?: ReactNode;
  className?: string;
}) {
  return (
    <header className={cn('flex shrink-0 items-start gap-3 px-8 pt-7 pb-4', className)}>
      {leading}
      <div className="min-w-0 flex-1">
        <h1 className="truncate text-2xl font-semibold tracking-tight">{title}</h1>
        {subtitle ? <p className="mt-1 truncate text-sm text-muted">{subtitle}</p> : null}
      </div>
      {actions ? <div className="flex shrink-0 items-center gap-2">{actions}</div> : null}
    </header>
  );
}

export function ErrorNotice({
  title,
  message,
  onRetry,
  retryLabel = '重试',
  className,
}: {
  title?: string;
  message: string;
  onRetry?: () => void;
  retryLabel?: string;
  className?: string;
}) {
  return (
    <Alert status="danger" className={cn('items-start', className)}>
      <Alert.Indicator />
      <Alert.Content>
        {title ? <Alert.Title>{title}</Alert.Title> : null}
        <Alert.Description className="selectable">{message}</Alert.Description>
        {onRetry ? (
          <div className="mt-2">
            <Button size="sm" variant="secondary" onPress={onRetry}>
              {retryLabel}
            </Button>
          </div>
        ) : null}
      </Alert.Content>
    </Alert>
  );
}

export function EmptyBlock({
  icon,
  title,
  description,
  action,
}: {
  icon?: ReactNode;
  title: string;
  description?: string;
  action?: ReactNode;
}) {
  return (
    <div className="flex flex-col items-center justify-center gap-2 py-16 text-center">
      {icon ? <div className="mb-1 text-muted">{icon}</div> : null}
      <p className="text-base font-medium">{title}</p>
      {description ? <p className="max-w-md text-sm text-muted">{description}</p> : null}
      {action ? <div className="mt-2">{action}</div> : null}
    </div>
  );
}

export function LoadingBlock({ label = '正在加载…' }: { label?: string }) {
  return (
    <div className="flex items-center justify-center gap-3 py-16 text-sm text-muted">
      <Spinner size="sm" color="current" />
      {label}
    </div>
  );
}

export function InlineWarning({ children }: { children: ReactNode }) {
  return (
    <div className="flex items-center gap-2 text-sm text-warning">
      <CircleAlert size={16} />
      <span>{children}</span>
    </div>
  );
}

/** A checkbox with an optional label to its right. */
export function CheckBox({
  isSelected,
  onChange,
  isDisabled,
  isIndeterminate,
  label,
  ariaLabel,
  className,
}: {
  isSelected: boolean;
  onChange: (selected: boolean) => void;
  isDisabled?: boolean;
  isIndeterminate?: boolean;
  label?: ReactNode;
  ariaLabel?: string;
  className?: string;
}) {
  return (
    <Checkbox
      isSelected={isSelected}
      onChange={onChange}
      isDisabled={isDisabled}
      isIndeterminate={isIndeterminate}
      aria-label={ariaLabel}
      className={className}
    >
      <Checkbox.Content>
        <Checkbox.Control>
          <Checkbox.Indicator />
        </Checkbox.Control>
        {label ? <Label>{label}</Label> : null}
      </Checkbox.Content>
    </Checkbox>
  );
}

export function Toggle({
  isSelected,
  onChange,
  isDisabled,
  label,
  ariaLabel,
}: {
  isSelected: boolean;
  onChange: (selected: boolean) => void;
  isDisabled?: boolean;
  label?: ReactNode;
  ariaLabel?: string;
}) {
  return (
    <Switch isSelected={isSelected} onChange={onChange} isDisabled={isDisabled} aria-label={ariaLabel}>
      <Switch.Content>
        <Switch.Control>
          <Switch.Thumb />
        </Switch.Control>
        {label ? <Label>{label}</Label> : null}
      </Switch.Content>
    </Switch>
  );
}

/** A settings row: a label and help text on the left, the control on the right. */
export function SettingRow({
  title,
  description,
  children,
  className,
}: {
  title: ReactNode;
  description?: ReactNode;
  children?: ReactNode;
  className?: string;
}) {
  return (
    <div className={cn('flex items-start justify-between gap-6 py-3', className)}>
      <div className="min-w-0 flex-1">
        <div className="text-sm font-medium">{title}</div>
        {description ? <div className="mt-0.5 text-xs leading-relaxed text-muted">{description}</div> : null}
      </div>
      {children ? <div className="flex shrink-0 items-center gap-2">{children}</div> : null}
    </div>
  );
}
