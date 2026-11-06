import { selectWebcamSource } from "../../../media-inputs";

export default {
  meta: {
    type: "2d",
    name: "Webcam",
    author: "2xAA",
    version: "1.0.0",
  },

  props: {
    scale: {
      type: "float",
      min: 0,
      max: 10,
      default: 1,
    },

    position: {
      type: "vec2",
      default: [0.5, 0.5],
      min: 0,
      max: 1,
    },

    imageSmoothing: {
      type: "bool",
      default: true,
    },

    camera: {
      type: "int",
      label: "Camera",
      min: 1,
      max: 8,
      default: 1,
    },
  },

  draw({ canvas, context, video, videos = [], props }) {
    const { position, scale, imageSmoothing, camera } = props;
    const source = selectWebcamSource(videos, camera, video);
    const videoSource = source?.canvas || source;

    if (!videoSource) {
      return;
    }

    const { width: videoWidth, height: videoHeight } = videoSource;
    const { width, height } = canvas;

    context.imageSmoothingEnabled = imageSmoothing;

    context.drawImage(
      videoSource,
      width * position[0] - (videoWidth * scale) / 2,
      height * (1 - position[1]) - (videoHeight * scale) / 2,
      videoWidth * scale,
      videoHeight * scale,
    );
  },
};
