#version 450

// Fullscreen-triangle vertex shader.
// A single oversized triangle covers the whole screen; positions and UVs are
// derived from gl_VertexIndex, so no vertex buffers are bound.
//   Vertex 0: pos (-1, -1), UV (0, 0)
//   Vertex 1: pos ( 3, -1), UV (2, 0)
//   Vertex 2: pos (-1,  3), UV (0, 2)

layout(location = 0) out vec2 fragUV;

void main() {
    vec2 positions[3] = vec2[](
        vec2(-1.0, -1.0),
        vec2(3.0, -1.0),
        vec2(-1.0, 3.0)
    );

    vec2 uvs[3] = vec2[](
        vec2(0.0, 0.0),
        vec2(2.0, 0.0),
        vec2(0.0, 2.0)
    );

    gl_Position = vec4(positions[gl_VertexIndex], 0.0, 1.0);
    fragUV = uvs[gl_VertexIndex];
}
