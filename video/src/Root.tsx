import { Composition } from "remotion";
import { Launch, FPS, DURATION } from "./Launch";

export const Root = () => (
  <Composition id="Launch" component={Launch} durationInFrames={DURATION} fps={FPS} width={1920} height={1080} />
);
