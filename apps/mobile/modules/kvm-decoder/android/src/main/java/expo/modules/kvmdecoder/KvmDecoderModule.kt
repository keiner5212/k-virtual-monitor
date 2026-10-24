package expo.modules.kvmdecoder

import android.content.Context
import android.graphics.SurfaceTexture
import android.media.MediaCodec
import android.media.MediaFormat
import android.os.Build
import android.util.Base64
import android.view.Surface
import android.view.TextureView
import expo.modules.kotlin.AppContext
import expo.modules.kotlin.modules.Module
import expo.modules.kotlin.modules.ModuleDefinition
import expo.modules.kotlin.viewevent.EventDispatcher
import expo.modules.kotlin.views.ExpoView
import java.nio.ByteBuffer

class KvmDecoderModule : Module() {
    override fun definition() = ModuleDefinition {
        Name("KvmDecoder")

        Function("isAvailable") {
            true
        }

        View(KvmVideoView::class) {
            Events("onDecodeError")
            AsyncFunction("pushFrame") { view: KvmVideoView, payload: String, keyframe: Boolean, width: Int, height: Int ->
                view.pushFrame(payload, keyframe, width, height)
            }
        }
    }
}

class KvmVideoView(context: Context, appContext: AppContext) :
    ExpoView(context, appContext),
    TextureView.SurfaceTextureListener {

    private val onDecodeError by EventDispatcher<Map<String, String>>()
    private val texture = TextureView(context)
    private val lock = Any()
    private var surface: Surface? = null
    private var codec: MediaCodec? = null
    private var codecWidth = 0
    private var codecHeight = 0
    private var reported = false
    private var ptsUs = 0L

    init {
        texture.layoutParams = LayoutParams(LayoutParams.MATCH_PARENT, LayoutParams.MATCH_PARENT)
        addView(texture)
        texture.surfaceTextureListener = this
    }

    fun pushFrame(payload: String, keyframe: Boolean, width: Int, height: Int) {
        val bytes = try {
            Base64.decode(payload, Base64.DEFAULT)
        } catch (_: IllegalArgumentException) {
            report("the frame was not valid base64")
            return
        }
        synchronized(lock) {
            val current = surface
            if (current == null || !current.isValid) {
                return
            }
            if (codec == null || width != codecWidth || height != codecHeight) {
                if (!keyframe) {
                    return
                }
                if (!startCodec(current, bytes, width, height)) {
                    return
                }
            }
            queue(bytes, keyframe)
        }
    }

    override fun onSurfaceTextureAvailable(texture: SurfaceTexture, width: Int, height: Int) {
        synchronized(lock) {
            surface?.release()
            surface = Surface(texture)
        }
    }

    override fun onSurfaceTextureSizeChanged(texture: SurfaceTexture, width: Int, height: Int) = Unit

    override fun onSurfaceTextureDestroyed(texture: SurfaceTexture): Boolean {
        synchronized(lock) {
            releaseCodec()
            surface?.release()
            surface = null
        }
        return false
    }

    override fun onSurfaceTextureUpdated(texture: SurfaceTexture) = Unit

    private fun startCodec(target: Surface, bytes: ByteArray, width: Int, height: Int): Boolean {
        releaseCodec()
        val sps = nal(bytes, 7)
        val pps = nal(bytes, 8)
        if (sps == null || pps == null) {
            report("the H.264 stream has no decoder config")
            return false
        }
        return try {
            val format = MediaFormat.createVideoFormat(MediaFormat.MIMETYPE_VIDEO_AVC, width, height)
            format.setByteBuffer("csd-0", ByteBuffer.wrap(sps))
            format.setByteBuffer("csd-1", ByteBuffer.wrap(pps))
            format.setInteger(MediaFormat.KEY_MAX_INPUT_SIZE, bytes.size.coerceAtLeast(width * height))
            if (Build.VERSION.SDK_INT >= 30) {
                format.setInteger(MediaFormat.KEY_LOW_LATENCY, 1)
            }
            val decoder = MediaCodec.createDecoderByType(MediaFormat.MIMETYPE_VIDEO_AVC)
            decoder.configure(format, target, null, 0)
            decoder.start()
            codec = decoder
            codecWidth = width
            codecHeight = height
            texture.surfaceTexture?.setDefaultBufferSize(width, height)
            true
        } catch (error: Exception) {
            releaseCodec()
            report(error.message ?: "H.264 decoder failed")
            false
        }
    }

    private fun queue(bytes: ByteArray, keyframe: Boolean) {
        val decoder = codec ?: return
        val index = decoder.dequeueInputBuffer(10_000)
        if (index < 0) {
            return
        }
        val input = decoder.getInputBuffer(index)
        if (input == null) {
            decoder.queueInputBuffer(index, 0, 0, ptsUs, 0)
            return
        }
        input.clear()
        if (bytes.size > input.capacity()) {
            decoder.queueInputBuffer(index, 0, 0, ptsUs, 0)
            return
        }
        input.put(bytes)
        ptsUs += 33_333
        val flags = if (keyframe) MediaCodec.BUFFER_FLAG_KEY_FRAME else 0
        decoder.queueInputBuffer(index, 0, bytes.size, ptsUs, flags)
        val info = MediaCodec.BufferInfo()
        while (true) {
            val output = decoder.dequeueOutputBuffer(info, 0)
            if (output >= 0) {
                decoder.releaseOutputBuffer(output, true)
            } else {
                break
            }
        }
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
            return bytes.copyOfRange(start, end)
        }
        return null
    }
}
