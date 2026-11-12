import type { ComponentChildren } from 'preact';

export function SettingsSection({ title, description, actions, children }: {
  title: string;
  description?: string;
  actions?: ComponentChildren;
  children: ComponentChildren;
}) {
  return (
    <section className='settings-section'>
      <header>
        <div>
          <h2>{title}</h2>
          {description && <p>{description}</p>}
        </div>
        {actions && <div className='settings-header-actions'>{actions}</div>}
      </header>
      {children}
    </section>
  );
}
