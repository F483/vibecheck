"""Export CLAP's audio side as one ONNX graph for the native app.

    uv run python -I scripts/export_clap_onnx.py <out.onnx> [--fp32]

10 s of 48 kHz mono PCM in, the unit-length 512-d embedding out. The feature
extraction (STFT as a convolution against a fixed DFT basis, slaney mel, dB)
lives inside the graph, so the app only decodes and resamples. Its output
matches the torch pipeline to 1e-6, checked on the cached embeddings.

The weights are stored in half precision (145 MB instead of 284 MB): each
large float32 weight becomes float16 plus a Cast back to float32 at the top of
the graph, so the arithmetic stays float32. The DFT basis and mel filters stay
float32; halving those moved one track's vector visibly. Against full
precision on 500 tracks: cosine >= 0.9999995, 499/500 identical decisions,
the one other a tie (core/examples/precision_check.rs). --fp32 keeps it all
full precision.

The Rust core looks for it at ~/Library/Application Support/vibecheck/models/
clap_audio.onnx on macOS (or $VIBECHECK_MODELS).
"""
import sys, numpy as np, onnx, torch
from onnx import helper, numpy_helper, TensorProto
from transformers import AutoProcessor, ClapModel

M = 'laion/larger_clap_music_and_speech'
OUT = sys.argv[1]
FP32 = '--fp32' in sys.argv[2:]
fe = AutoProcessor.from_pretrained(M, local_files_only=True).feature_extractor
model = ClapModel.from_pretrained(M, local_files_only=True).eval()
N, HOP = fe.fft_window_size, fe.hop_length          # 1024, 480
win = np.hanning(N + 1)[:-1]                        # periodic hann, as window_function
k = np.arange(N // 2 + 1)[:, None]; n = np.arange(N)[None, :]
ang = 2 * np.pi * k * n / N
basis = np.concatenate([np.cos(ang) * win, -np.sin(ang) * win])[:, None, :]   # (1026,1,1024)

class Wrap(torch.nn.Module):
    def __init__(self):
        super().__init__()
        self.basis = torch.nn.Parameter(torch.tensor(basis, dtype=torch.float32), requires_grad=False)
        self.mel = torch.nn.Parameter(torch.tensor(fe.mel_filters_slaney, dtype=torch.float32), requires_grad=False)
        self.m = model
    def forward(self, pcm):                                  # (B, 480000)
        x = torch.nn.functional.pad(pcm[:, None, :], (N // 2, N // 2), mode='reflect')
        z = torch.nn.functional.conv1d(x, self.basis, stride=HOP)   # (B,1026,1001)
        re, im = z[:, :N // 2 + 1], z[:, N // 2 + 1:]
        p = re * re + im * im                                # (B,513,1001)
        mel = torch.einsum('bft,fm->btm', p, self.mel)
        db = 10.0 * torch.log10(torch.clamp(mel, min=1e-10))
        return self.m.get_audio_features(input_features=db[:, None]).pooler_output

w = Wrap().eval()
ex = torch.zeros(2, 480000)
with torch.inference_mode():
    torch.onnx.export(w, (ex,), OUT, input_names=['pcm'], output_names=['embedding'],
                      dynamic_axes={'pcm': {0: 'batch'}, 'embedding': {0: 'batch'}},
                      opset_version=18, dynamo=False)

def half_weights(path):
    m = onnx.load(path)
    g = m.graph
    casts = []
    for init in list(g.initializer):
        a = numpy_helper.to_array(init)
        if a.dtype != np.float32 or a.size < 1024 or init.name in ('basis', 'mel'):
            continue
        g.initializer.remove(init)
        g.initializer.append(numpy_helper.from_array(a.astype(np.float16), init.name + '__fp16'))
        casts.append(helper.make_node('Cast', [init.name + '__fp16'], [init.name],
                                      to=TensorProto.FLOAT, name=init.name + '__to_fp32'))
    for c in reversed(casts):
        g.node.insert(0, c)
    onnx.checker.check_model(m)
    onnx.save(m, path)
    return len(casts)

if not FP32:
    print(half_weights(OUT), 'weights stored as float16')
print('wrote', OUT)
