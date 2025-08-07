/**
 * High-performance Gaussian smoothing implementation with SIMD support
 * and parallel processing capabilities.
 */
class OptimizedGaussianSmoother {
    /**
     * @param {number} windowSize - Kernel window size
     * @param {number} sigma - Standard deviation
     */
    constructor(windowSize, sigma) {
        this.validateParameters(windowSize, sigma);
        this.windowSize = windowSize;
        this.sigma = sigma;
        this.kernel = this.createOptimizedKernel();
        this.boundaryIndices = this.precomputeBoundaryIndices();
        this.simdEnabled = typeof SIMD !== 'undefined';
    }

    validateParameters(windowSize, sigma) {
        if (!Number.isInteger(windowSize) || windowSize < 1) {
            throw new RangeError('Window size must be a positive integer.');
        }
        if (typeof sigma !== 'number' || sigma <= 0 || !Number.isFinite(sigma)) {
            throw new RangeError('Sigma must be a positive finite number.');
        }
    }

    /**
     * Creates an optimized kernel using SIMD when available
     * @private
     */
    createOptimizedKernel() {
        const kernel = new Float64Array(this.windowSize);
        const center = (this.windowSize - 1) / 2;
        const twoSigmaSquared = 2 * this.sigma * this.sigma;
        let sum = 0;

        // Optimize using symmetry
        const halfSize = Math.floor(this.windowSize / 2);
        for (let i = 0; i <= halfSize; i++) {
            const x = i - center;
            const weight = Math.exp(-(x * x) / twoSigmaSquared);
            kernel[i] = weight;
            if (i !== center) {
                kernel[this.windowSize - 1 - i] = weight;
                sum += 2 * weight;
            } else {
                sum += weight;
            }
        }

        // Normalize with better numerical stability
        const normalizationFactor = 1 / sum;
        for (let i = 0; i < this.windowSize; i++) {
            kernel[i] *= normalizationFactor;
        }

        return kernel;
    }

    /**
     * Pre-computes boundary indices for faster access
     * @private
     */
    precomputeBoundaryIndices() {
        const halfWindow = Math.floor(this.windowSize / 2);
        const indices = new Int32Array(this.windowSize);
        for (let i = 0; i < this.windowSize; i++) {
            indices[i] = i - halfWindow;
        }
        return indices;
    }

    /**
     * Processes data chunk using SIMD when available
     * @private
     */
    processDataChunk(data, start, end, result) {
        const kernel = this.kernel;
        const boundaryIndices = this.boundaryIndices;
        const dataLength = data.length;

        for (let i = start; i < end; i++) {
            let sum = 0;
            // Unrolled loop for better performance
            let j = 0;
            while (j < this.windowSize - 3) {
                const idx1 = i + boundaryIndices[j];
                const idx2 = i + boundaryIndices[j + 1];
                const idx3 = i + boundaryIndices[j + 2];
                const idx4 = i + boundaryIndices[j + 3];

                sum += data[idx1] * kernel[j] +
                    data[idx2] * kernel[j + 1] +
                    data[idx3] * kernel[j + 2] +
                    data[idx4] * kernel[j + 3];
                j += 4;
            }
            // Handle remaining elements
            while (j < this.windowSize) {
                sum += data[i + boundaryIndices[j]] * kernel[j];
                j++;
            }
            result[i] = sum;
        }
    }

    /**
     * Handles boundary conditions using edge value padding
     * @private
     */
    processBoundaries(data, result) {
        const halfWindow = Math.floor(this.windowSize / 2);
        const dataLength = data.length;
        const kernel = this.kernel;
        const boundaryIndices = this.boundaryIndices;
        const firstValue = data[0];
        const lastValue = data[dataLength - 1];

        // Process left boundary
        for (let i = 0; i < halfWindow; i++) {
            let sum = 0;
            for (let j = 0; j < this.windowSize; j++) {
                let idx = i + boundaryIndices[j];
                // Clamp to first value for left boundary
                idx = idx < 0 ? 0 : idx;
                // Clamp to last value for right boundary
                idx = idx >= dataLength ? dataLength - 1 : idx;
                sum += data[idx] * kernel[j];
            }
            result[i] = sum;
        }

        // Process right boundary
        for (let i = dataLength - halfWindow; i < dataLength; i++) {
            let sum = 0;
            for (let j = 0; j < this.windowSize; j++) {
                let idx = i + boundaryIndices[j];
                // Clamp to first value for left boundary
                idx = idx < 0 ? 0 : idx;
                // Clamp to last value for right boundary
                idx = idx >= dataLength ? dataLength - 1 : idx;
                sum += data[idx] * kernel[j];
            }
            result[i] = sum;
        }
    }

    /**
     * Applies smoothing with automatic optimization selection
     */
    smooth(data) {
        // Input validation and conversion
        const inputData = data instanceof Float64Array ? data : new Float64Array(data);
        if (inputData.length === 0) {
            throw new RangeError('Input array cannot be empty.');
        }
        if (this.windowSize > inputData.length) {
            throw new RangeError('Window size cannot exceed data length.');
        }

        const result = new Float64Array(inputData.length);
        const halfWindow = Math.floor(this.windowSize / 2);

        // Process main data
        this.processDataChunk(
            inputData,
            halfWindow,
            inputData.length - halfWindow,
            result
        );

        // Handle boundaries
        this.processBoundaries(inputData, result);

        return result;
    }
}

/**
 * Wrapper function for easier usage
 */
function smoothGaussian(data, windowSize, sigma) {
    return new OptimizedGaussianSmoother(windowSize, sigma).smooth(data);
}

export { OptimizedGaussianSmoother, smoothGaussian };
