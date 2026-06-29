import { createFileRoute } from '@tanstack/react-router';

const Artists = () => {
  return <h1>Исполнители</h1>;
};

export const Route = createFileRoute('/artists')({
  component: Artists,
});
