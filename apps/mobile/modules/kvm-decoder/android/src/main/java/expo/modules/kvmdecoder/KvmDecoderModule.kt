package expo.modules.kvmdecoder

import android.content.Context
import android.graphics.SurfaceTexture
import android.media.MediaCodec
import android.media.MediaFormat
import android.os.Build
import android.view.Surface
import android.view.TextureView
import expo.modules.kotlin.AppContext
import expo.modules.kotlin.modules.Module
import expo.modules.kotlin.modules.ModuleDefinition
import expo.modules.kotlin.viewevent.EventDispatcher
import expo.modules.kotlin.views.ExpoView
import java.nio.ByteBuffer
import java.util.concurrent.locks.ReentrantLock
import kotlin.concurrent.withLock

/** How long the decoder may wait for a free input buffer, in microseconds. */
private const val INPUT_TIMEOUT_US = 3000L

/** How long one step of the drain may wait, in microseconds. */
private const val DRAIN_TIMEOUT_US = 2000L

/** How long the drain may take in total, in nanoseconds. */
private const val DRAIN_BUDGET_NS = 10_000_000L

/** How many pictures may wait for the decoder. */
private const val MAX_PENDING = 4

class KvmDecoderModule : Module() {
    override fun definition() = ModuleDefinition {
        Name("KvmDecoder")

        Function("isAvailable") {
            true
        }

        View(KvmVideoView::class) {
            Events("onDecodeError")
            AsyncFunction("pushFrame") { view: KvmVideoView, payload: ByteArray, keyframe: Boolean, width: Int, height: Int, ptsMs: Double ->
                view.pushFrame(payload, keyframe, width, height, ptsMs.toLong())
            }
        }
    }
}

/** A picture that waits for the decoder. */
private class Picture(
    val bytes: ByteArray,
    val keyframe: Boolean,
    val width: Int,
    val height: Int,
    val ptsUs: Long,
)

/**
 * The decoder of the phone.
 *
 * The pictures are decoded by a thread of their own, in the order they come
 * in: a picture that depends on another one cannot be drawn when that other
 * one was lost, so the decoder waits for a key frame when the chain breaks.
 */
class KvmVideoView(context: Context, appContext: AppContext) :
    ExpoView(context, appContext),
    TextureView.SurfaceTextureListener {

    private val onDecodeError by EventDispatcher<Map<String, String>>()
    private val texture = TextureView(context)
    private val lock = ReentrantLock()
    private val signal = lock.newCondition()
    private val pending = ArrayDeque<Picture>()
    private var worker: Thread? = null
    private var running = false
    private var surface: Surface? = null
    private var codec: MediaCodec? = null
    private var codecWidth = 0
    private var codecHeight = 0
    /** True when a picture was lost, so the decoder has to start again. */
    private var broken = false
    private var reported = false

    init {
        texture.layoutParams = LayoutParams(LayoutParams.MATCH_PARENT, LayoutParams.MATCH_PARENT)
        addView(texture)
        texture.surfaceTextureListener = this
        startWorker()
    }

    /**
     * Takes a picture of the stream.
     *
     * The picture waits for the decoder. It never waits here, so the phone is
     * not held back by the decoder.
     */
    fun pushFrame(payload: ByteArray, keyframe: Boolean, width: Int, height: Int, ptsMs: Long) {
        val picture = Picture(payload, keyframe, width, height, ptsMs * 1000)
        lock.withLock {
            pending.addLast(picture)
            // A picture that waits too long is worth less than the one that
            // replaced it, but the chain of the decoder breaks with it.
            while (pending.size > MAX_PENDING) {
                pending.removeFirst()
                broken = true
            }
            signal.signalAll()
        }
    }

    private fun startWorker() {
        if (worker != null) {
            return
        }
        running = true
        worker = Thread({ decodeLoop() }, "kvm-decoder").apply { start() }
    }

    private fun decodeLoop() {
        while (true) {
            val picture = nextPicture() ?: return
            decode(picture)
        }
    }

    /** The next picture, or null when the decoder has to stop. */
    private fun nextPicture(): Picture? {
        lock.withLock {
            while (running && pending.isEmpty()) {
                try {
                    signal.await()
                } catch (_: InterruptedException) {
                    return null
                }
            }
            if (!running) {
                return null
            }
            return pending.removeFirst()
        }
    }

    private fun decode(picture: Picture) {
        lock.withLock {
            val target = surface
            if (target == null || !target.isValid) {
                // Without a surface there is nothing to draw on, so the
                // pictures are lost and the chain of the decoder breaks.
                broken = true
                return
            }
            if (broken) {
                // A picture was lost. The decoder recovers with the next key
                // frame, which starts a new sequence of its own.
                if (!picture.keyframe) {
                    return
                }
                broken = false
            }
            if (codec == null || picture.width != codecWidth || picture.height != codecHeight) {
                // The decoder cannot start with a picture that depends on
                // another one.
                if (!picture.keyframe) {
                    return
                }
                if (!startCodec(target, picture)) {
                    return
                }
            }
            queueToCodec(picture)
        }
    }

    override fun onSurfaceTextureAvailable(texture: SurfaceTexture, width: Int, height: Int) {
        lock.withLock {
            // The surface of the old decoder is gone, so it cannot be used.
            releaseCodec()
            broken = false
            surface?.release()
            surface = Surface(texture)
            signal.signalAll()
        }
    }

    override fun onSurfaceTextureSizeChanged(texture: SurfaceTexture, width: Int, height: Int) = Unit

    override fun onSurfaceTextureDestroyed(texture: SurfaceTexture): Boolean {
        lock.withLock {
            releaseCodec()
            surface?.release()
            surface = null
        }
        return false
    }

    override fun onSurfaceTextureUpdated(texture: SurfaceTexture) = Unit

    override fun onDetachedFromWindow() {
        lock.withLock {
            running = false
            signal.signalAll()
        }
        worker?.join(1000)
        worker = null
        releaseCodec()
        super.onDetachedFromWindow()
    }

    override fun onAttachedToWindow() {
        super.onAttachedToWindow()
        startWorker()
    }

    private fun startCodec(target: Surface, picture: Picture): Boolean {
        releaseCodec()
        val sps = nal(picture.bytes, 7)
        val pps = nal(picture.bytes, 8)
        if (sps == null || pps == null) {
            report("the H.264 stream has no decoder config")
            return false
        }
        return try {
            val format = MediaFormat.createVideoFormat(
                MediaFormat.MIMETYPE_VIDEO_AVC,
                picture.width,
                picture.height,
            )
            // The configuration of the codec is the parameter set without the
            // start code: the start code belongs to the stream, not to the
            // configuration.
            format.setByteBuffer("csd-0", ByteBuffer.wrap(sps))
            format.setByteBuffer("csd-1", ByteBuffer.wrap(pps))
            format.setInteger(
                MediaFormat.KEY_MAX_INPUT_SIZE,
                picture.bytes.size.coerceAtLeast(picture.width * picture.height),
            )
            if (Build.VERSION.SDK_INT >= 30) {
                format.setInteger(MediaFormat.KEY_LOW_LATENCY, 1)
            }
            val decoder = MediaCodec.createDecoderByType(MediaFormat.MIMETYPE_VIDEO_AVC)
            decoder.configure(format, target, null, 0)
            decoder.start()
            codec = decoder
            codecWidth = picture.width
            codecHeight = picture.height
            texture.surfaceTexture?.setDefaultBufferSize(picture.width, picture.height)
            reported = false
            true
        } catch (error: Exception) {
            releaseCodec()
            report(error.message ?: "the H.264 decoder did not start")
            false
        }
    }

    private fun queueToCodec(picture: Picture) {
        val decoder = codec ?: return
        val index = try {
            decoder.dequeueInputBuffer(INPUT_TIMEOUT_US)
        } catch (error: MediaCodec.CodecException) {
            failed(error)
            return
        }
        if (index < 0) {
            // The decoder is busy. This picture is late, so it is dropped and
            // the chain breaks.
            broken = true
            return
        }
        val input = decoder.getInputBuffer(index)
        if (input == null) {
            decoder.queueInputBuffer(index, 0, 0, picture.ptsUs, 0)
            return
        }
        input.clear()
        if (picture.bytes.size > input.capacity()) {
            decoder.queueInputBuffer(index, 0, 0, picture.ptsUs, 0)
            broken = true
            report("the picture is larger than the decoder can take")
            return
        }
        input.put(picture.bytes)
        val flags = if (picture.keyframe) MediaCodec.BUFFER_FLAG_KEY_FRAME else 0
        decoder.queueInputBuffer(index, 0, picture.bytes.size, picture.ptsUs, flags)
        drain(decoder)
    }

    /** Frees the pictures the decoder has ready. */
    private fun drain(decoder: MediaCodec) {
        val info = MediaCodec.BufferInfo()
        val deadline = System.nanoTime() + DRAIN_BUDGET_NS
        while (true) {
            val timeout = if (System.nanoTime() < deadline) DRAIN_TIMEOUT_US else 0L
            val output = try {
                decoder.dequeueOutputBuffer(info, timeout)
            } catch (error: MediaCodec.CodecException) {
                failed(error)
                return
            }
            if (output < 0) {
                return
            }
            if ((info.flags and MediaCodec.BUFFER_FLAG_CODEC_CONFIG) != 0) {
                decoder.releaseOutputBuffer(output, false)
                continue
            }
            decoder.releaseOutputBuffer(output, true)
        }
    }

    /**
     * The decoder failed.
     *
     * The codec is broken from now on, so it goes away and the next key
     * frame starts a new one.
     */
    private fun failed(error: MediaCodec.CodecException) {
        broken = true
        releaseCodec()
        report("the decoder of the phone failed: ${error.diagnosticInfo}")
    }

    private fun releaseCodec() {
        val decoder = codec ?: return
        codec = null
        try {
            decoder.stop()
        } catch (_: Exception) {
        }
        try {
            decoder.release()
        } catch (_: Exception) {
        }
    }

    private fun report(message: String) {
        if (reported) {
            return
        }
        reported = true
        val payload = mapOf("message" to message)
        post { onDecodeError(payload) }
    }

    /**
     * The NAL of the given type, without the start code.
     *
     * The decoder wants the parameter sets without it: the start code tells
     * where a NAL begins in the stream, not what the NAL holds.
     */
    private fun nal(bytes: ByteArray, kind: Int): ByteArray? {
        val starts = ArrayList<Int>()
        var index = 0
        while (index + 3 < bytes.size) {
            if (bytes[index] == 0.toByte() && bytes[index + 1] == 0.toByte() && bytes[index + 2] == 1.toByte()) {
                starts.add(index)
                index += 3
                continue
            }
            if (index + 4 < bytes.size &&
                bytes[index] == 0.toByte() &&
                bytes[index + 1] == 0.toByte() &&
                bytes[index + 2] == 0.toByte() &&
                bytes[index + 3] == 1.toByte()
            ) {
                starts.add(index)
                index += 4
                continue
            }
            index += 1
        }
        for (position in starts.indices) {
            val start = starts[position]
            val header = if (bytes[start + 2] == 1.toByte()) start + 3 else start + 4
            if (header >= bytes.size) {
                continue
            }
            if ((bytes[header].toInt() and 0x1f) != kind) {
                continue
            }
            val end = if (position + 1 < starts.size) starts[position + 1] else bytes.size
            // The zeros at the end of a NAL belong to the next start code.
            var stop = end
            while (stop > header && bytes[stop - 1] == 0.toByte()) {
                stop -= 1
            }
            return bytes.copyOfRange(header, stop)
        }
        return null
    }
}
