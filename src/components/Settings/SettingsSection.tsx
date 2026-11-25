import type { ComponentChildren } from 'preact';
import { BetaBadge } from '@/components/Settings/BetaBadge';

export function SettingsSection({ title, beta, description, actions, children }: {
  title: string;
  beta?: boolean;
  description?: string;
  actions?: ComponentChildren;
  children: ComponentChildren;
}) {
  return (
    <section className='settings-section'>
      <header>
        <div>
          <h2>{title}{beta && <BetaBadge />}</h2>
          {description && <p>{description}</p>}
        </div>
        {actions && <div className='settings-header-actions'>{actions}</div>}
      </header>
      {children}
    </section>
  );
}
